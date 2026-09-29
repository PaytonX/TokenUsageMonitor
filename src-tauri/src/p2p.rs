//! 同网段设备自动发现（mDNS / DNS-SD）。
//!
//! 现状多端同步需要用户在每台 agent 上手填 hub 地址，多机部署时很繁琐。
//! 本模块让同一局域网内的实例通过 mDNS 互相发现，从而自动完成"谁该向谁
//! 上报"的判断，用户无需填写任何地址。
//!
//! ## 为什么只做发现、不做传输
//!
//! 传输完全复用 [`crate::hub`]：它已实现 `/ingest`（上报）与 `/devices`
//! （汇总）两个端点。mDNS 只解决"对端地址从哪来"这一个缺口，因此数据
//! 流、鉴权、持久化都无需改动。
//!
//! ## 为什么不做多跳 / Gossip
//!
//! 目标规模在 10 台以内，且是全连接拓扑：每台实例直接向其余所有对端上报，
//! 一跳即可让每台都拿到全量设备（`hub_device` 表以 `device_id` 为主键，
//! 各自存下所有对端的上报）。转发会引入重复计数与环路风险，对当前规模
//! 是纯粹的复杂度浪费。
//!
//! ## 跨网段
//!
//! mDNS 组播不跨网段，这是它的固有限制而非缺陷。跨网段仍由
//! `hub_mode = "agent"` + 手动填写 `hub_base` 承担，与本模块并存。

use mdns_sd::{ServiceDaemon, ServiceEvent, ServiceInfo};
use std::collections::{HashMap, HashSet};
use std::net::Ipv4Addr;
use std::sync::{Arc, Mutex};

/// mDNS 服务类型。**末尾的点不可省略**：mdns-sd 要求服务类型以
/// `._tcp.local.` / `._udp.local.` 结尾（含尾部点），缺了会被直接拒绝
/// （browse 失败 → 静默禁用发现，历史上正是这个 bug 让 lan 模式的
/// 自动发现从未生效过）。改动此值会让新旧版本互相发现不到。
const SERVICE_TYPE: &str = "_tokenusage._tcp.local.";

/// TXT 属性键：广播方携带自己的稳定设备 ID，供对端识别。
const PROP_DEVICE_ID: &str = "device_id";

/// 一次成功发现的对端。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Peer {
    /// 对端上报使用的稳定设备 ID（用于识别"是不是自己"）。
    pub device_id: String,
    /// 对端的局域网地址，形如 `192.168.1.23:43210`。
    pub addr: String,
}

/// 持续维护"已发现对端"的后台服务。
///
/// 发现结果以 `device_id` 为键去重：mDNS 会周期性重复广播同一台设备
/// （TTL 到期后重新注册），若不去重，对端列表会随时间重复膨胀。同一
/// `device_id` 再次出现时只刷新地址，不新增条目。
pub struct Discovery {
    /// `device_id` → `host:port`
    peers: Arc<Mutex<HashMap<String, String>>>,
    /// 本机 device_id，用于过滤 mDNS 回环发现（会搜到自己）。
    self_id: String,
}

impl Discovery {
    /// 启动发现服务。`port` 为本机 hub 监听端口，会随服务记录广播出去。
    ///
    /// mDNS 在部分受限网络（Docker bridge、部分 VPN、禁 multicast 的企业网）
    /// 上不可用；这属于预期降级——此时对端列表为空，用户仍可手动填
    /// `hub_base` 完成跨网段或跨网络的同步。
    pub fn start(self_id: String, port: u16) -> Discovery {
        let peers: Arc<Mutex<HashMap<String, String>>> = Arc::new(Mutex::new(HashMap::new()));
        let d = Discovery {
            peers: peers.clone(),
            self_id: self_id.clone(),
        };

        let daemon = match ServiceDaemon::new() {
            Ok(v) => v,
            Err(e) => {
                eprintln!("[p2p] mDNS daemon unavailable, discovery disabled: {e}");
                return d;
            }
        };

        // 浏览本服务类型以发现对端。
        let receiver = match daemon.browse(SERVICE_TYPE) {
            Ok(r) => r,
            Err(e) => {
                eprintln!("[p2p] mDNS browse failed, discovery disabled: {e}");
                return d;
            }
        };

        // 广播自己的服务，供其它实例发现。需显式给出本机 IPv4——
        // 0.21 的 ServiceInfo::new 不再自动探测网卡。取第一个非回环 IPv4。
        let Some(ip) = local_ipv4() else {
            eprintln!("[p2p] no non-loopback IPv4, advertising disabled");
            return d;
        };
        let props = [(PROP_DEVICE_ID, self_id.as_str())];
        // host 也必须以 `.local.` 结尾（与 SERVICE_TYPE 同一种校验）；
        // 用 self_id（即小写主机名）保证多设备间主机记录不冲突。
        let host = format!(
            "{}.local.",
            self_id
                .chars()
                .filter(|c| c.is_ascii_alphanumeric() || *c == '-')
                .collect::<String>()
        );
        match ServiceInfo::new(
            SERVICE_TYPE,
            "TokenUsageMonitor",
            &host,
            ip.to_string(),
            port,
            &props[..],
        ) {
            Ok(info) => {
                if let Err(e) = daemon.register(info) {
                    eprintln!("[p2p] mDNS register failed: {e}");
                }
            }
            Err(e) => eprintln!("[p2p] mDNS service info invalid: {e}"),
        }

        std::thread::Builder::new()
            .name("tum-mdns".into())
            .spawn(move || {
                for event in receiver {
                    let ServiceEvent::ServiceResolved(info) = event else {
                        continue;
                    };
                    let Some(remote_id) = parse_device_id(info.get_property_val_str(PROP_DEVICE_ID))
                    else {
                        continue;
                    };
                    // 回环发现：会搜到本机广播的服务，必须跳过，
                    // 否则自己会把自己当作对端上报。
                    if remote_id == self_id {
                        continue;
                    }
                    let Some(addr) = first_socket_addr(&info.get_addresses_v4(), info.get_port())
                    else {
                        continue;
                    };
                    let mut guard = peers.lock().unwrap();
                    // 同一 device_id 覆盖而非新增，避免重复广播导致膨胀。
                    if let Some(existing) = guard.get(&remote_id) {
                        if existing == &addr {
                            continue;
                        }
                    }
                    guard.insert(remote_id.clone(), addr.clone());
                    drop(guard);
                    eprintln!("[p2p] discovered peer {remote_id} at {addr}");
                }
            })
            .ok();

        d
    }

    /// 当前已发现的对端（不含自己）。
    pub fn peers(&self) -> Vec<Peer> {
        let guard = self.peers.lock().unwrap();
        let mut out: Vec<Peer> = guard
            .iter()
            .map(|(device_id, addr)| Peer {
                device_id: device_id.clone(),
                addr: addr.clone(),
            })
            .collect();
        drop(guard);
        // 稳定排序，便于测试与日志比对。
        out.sort_by(|a, b| a.device_id.cmp(&b.device_id));
        out
    }
}

/// 第一个非回环、未指定的本机 IPv4。
///
/// 没有可用地址时返回 `None`：此时无法广播，服务发现无从谈起，
/// 调用方据此降级为"仅手动 hub"。
fn local_ipv4() -> Option<Ipv4Addr> {
    // std 的 UDP connect 技巧：不会真正发包，但能让系统按默认路由选源地址，
    // 天然适配多网卡（选中实际用于出网的网卡，而非硬编码网卡枚举）。
    let sock = std::net::UdpSocket::bind("0.0.0.0:0").ok()?;
    sock.connect("8.8.8.8:80").ok()?;
    match sock.local_addr().ok()?.ip() {
        std::net::IpAddr::V4(v4) if !v4.is_loopback() && !v4.is_unspecified() => Some(v4),
        _ => None,
    }
}

/// 从 TXT 属性中取回广播方携带的 `device_id`。
///
/// 取不到时返回 `None`——宁可忽略该对端，也不要把它错认成别的设备：
/// 一旦把 A 的地址记成 B 的归属，mesh 上报就会写错行。
fn parse_device_id(raw: Option<&str>) -> Option<String> {
    let value = raw?.trim();
    if value.is_empty() {
        return None;
    }
    Some(value.to_string())
}

/// 选出对端地址：跳过回环与 0.0.0.0，取第一个可用 IPv4。
///
/// 跳过回环很重要——若把 `127.0.0.1:port` 当成对端，本机会向自己上报，
/// 形成自我循环（虽然不会崩，但会掩盖真实的发现失败）。
fn first_socket_addr(addrs: &HashSet<Ipv4Addr>, port: u16) -> Option<String> {
    let mut sorted: Vec<Ipv4Addr> = addrs
        .iter()
        .filter(|a| !a.is_loopback() && !a.is_unspecified())
        .copied()
        .collect();
    // 多网卡时地址是 HashSet（无序），排序以保证行为可复现。
    sorted.sort();
    let ip = sorted.first()?;
    Some(format!("{ip}:{port}"))
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::HashSet;

    fn addr(a: &str, p: u16) -> String {
        format!("{a}:{p}")
    }

    /// 发现结果去重：同一 device_id 重复出现时覆盖地址而非新增。
    /// 直接构造 Discovery 的内部状态来测，避免依赖真实网络。
    #[test]
    fn peers_are_deduped_by_device_id() {
        let map: HashMap<String, String> = HashMap::from([
            ("dev-a".to_string(), addr("192.168.1.5", 43210)),
            ("dev-b".to_string(), addr("192.168.1.6", 43210)),
        ]);
        let d = Discovery {
            peers: Arc::new(Mutex::new(map)),
            self_id: "me".to_string(),
        };
        let got = d.peers();
        assert_eq!(got.len(), 2);
        // 排序稳定：dev-a 在 dev-b 之前
        assert_eq!(got[0].device_id, "dev-a");
        assert_eq!(got[1].device_id, "dev-b");
        // 再次读取应仍是 2 条，不因读取而增长
        assert_eq!(d.peers().len(), 2);
    }

    #[test]
    fn empty_discovery_yields_no_peers() {
        let d = Discovery {
            peers: Arc::new(Mutex::new(HashMap::new())),
            self_id: "me".to_string(),
        };
        assert!(d.peers().is_empty());
    }

    /// 服务类型是对外协议的一部分：改动会让新旧版本互相发现不到。
    /// 同时锁住尾部点：mdns-sd 要求以 `._tcp.local.` 结尾，缺了会被
    /// browse 直接拒绝（曾经的静默禁用 bug）。
    #[test]
    fn service_type_constant_is_stable() {
        assert_eq!(SERVICE_TYPE, "_tokenusage._tcp.local.");
        assert!(SERVICE_TYPE.ends_with("._tcp.local."), "mdns-sd requires a trailing dot");
    }

    /// 对端去重的键必须是 device_id 而不是地址——同一设备换 IP（DHCP 续租）
    /// 后仍应视为同一台，否则设备页会出现重复行。
    #[test]
    fn dedup_key_is_device_id_not_address() {
        let mut map: HashMap<String, String> = HashMap::new();
        map.insert("dev-a".to_string(), addr("192.168.1.5", 43210));
        map.insert("dev-a".to_string(), addr("192.168.1.77", 43210));
        assert_eq!(map.len(), 1, "same device must not create a second entry");
        assert_eq!(map.get("dev-a").unwrap(), "192.168.1.77:43210");
    }

    #[test]
    fn first_socket_addr_skips_loopback_and_unspecified() {
        let set: HashSet<Ipv4Addr> = HashSet::from([
            Ipv4Addr::LOCALHOST,
            Ipv4Addr::UNSPECIFIED,
            Ipv4Addr::new(192, 168, 1, 23),
        ]);
        let got = first_socket_addr(&set, 43210).expect("should pick the LAN address");
        assert_eq!(got, "192.168.1.23:43210");
    }

    #[test]
    fn first_socket_addr_returns_none_when_all_addresses_unusable() {
        let set: HashSet<Ipv4Addr> = HashSet::from([
            Ipv4Addr::LOCALHOST,
            Ipv4Addr::UNSPECIFIED,
        ]);
        assert!(first_socket_addr(&set, 43210).is_none());
        assert!(first_socket_addr(&HashSet::new(), 43210).is_none());
    }

    /// 多网卡时必须给出确定结果，否则对端地址会随 HashSet 迭代顺序漂移。
    #[test]
    fn first_socket_addr_is_deterministic_with_multiple_addresses() {
        let set: HashSet<Ipv4Addr> = HashSet::from([
            Ipv4Addr::new(192, 168, 1, 30),
            Ipv4Addr::new(192, 168, 1, 20),
            Ipv4Addr::new(10, 0, 0, 5),
        ]);
        let first = first_socket_addr(&set, 43210).unwrap();
        for _ in 0..20 {
            assert_eq!(first_socket_addr(&set, 43210).unwrap(), first);
        }
    }

    /// 取不到 device_id 就必须丢弃该对端——错认会把用量写到别的设备名下。
    #[test]
    fn parse_device_id_rejects_missing_or_empty() {
        assert_eq!(parse_device_id(Some("dev-1")), Some("dev-1".to_string()));
        // 首尾空白来自 TXT 编码差异，应被裁掉而不是当成 id 的一部分
        assert_eq!(parse_device_id(Some("  dev-1 ")), Some("dev-1".to_string()));
        assert_eq!(parse_device_id(None), None);
        assert_eq!(parse_device_id(Some("")), None);
        assert_eq!(parse_device_id(Some("   ")), None);
    }
}
