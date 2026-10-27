//! 本机 AI 工具 SQLite 库的健壮只读打开器（WSL / 9p 场景的副本回退）。
//!
//! 为什么需要这一层：本机 AI 工具的数据目录可能已经迁进 WSL，从 Windows 侧看到
//! 的路径是 `\\wsl$\<distro>\home\<user>\...`，底层走 9p 共享。这类库多是 WAL
//! 模式（实测 Hermes 的 `state.db` 文件头偏移 18/19 都是 2，旁边有 4.4MB 的
//! `-wal`），而 9p **不支持 SQLite 通过 `-shm` 做 WAL 锁协商**，于是从 Windows
//! 侧只读打开会直接失败：
//!
//! ```text
//! \\wsl$\Ubuntu-22.04\home\payton\.hermes\state.db（452MB，WAL 模式，有 -wal）
//! python: sqlite3.OperationalError: database is locked
//! ```
//!
//! 但把 `state.db` + `state.db-wal` **拷贝到本地临时目录**后，副本可以正常打开并
//! 查询（41 张表可读，`sessions` 等表都在）。所以这里先按仓库既有语义直接只读打开
//! （[`super::open_sqlite_readonly`]：只读 + URI，绝不 `immutable=1`）；打开或首次
//! 查询失败时，再把主库和 `-wal` 复制到一个唯一临时目录，在副本上做 WAL 恢复后以
//! 只读语义查询。副本连接 Drop 时连同临时目录一起清理。
//!
//! 为什么不要 `-shm`：见 [`SIDECARS`]——拷来的 WAL 索引与副本对不上，会让 SQLite
//! 以为"WAL 里没有新帧"，于是把空表当成真实结果。
//!
//! 边界与取舍：
//! - 不做缓存/增量：每次回退都重新拷贝，因为 WAL 随时在增长，旧副本会读到陈旧值
//!   （扫描结果走全量 REPLACE，陈旧快照会覆盖账本里正确的数字）。
//! - 不做 `immutable=1`：那个模式让 SQLite 完全忽略 `-wal`，读到上次 checkpoint
//!   的旧快照（见 `local::open_sqlite_readonly` 的注释）。
//! - 拷前有体积上限；拷贝失败、超时、副本也打不开都返回 `None`，绝不 panic。
//!   调用方本轮跳过该数据源，账本按来源批量删除，缺源等于保留上一轮旧行。

use rusqlite::Connection;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::time::Duration;

/// 回退复制的体积上限（主库 + 伴生文件合计）。超过就放弃，避免把磁盘/内存打爆。
const MAX_COPY_BYTES: u64 = 2 * 1024 * 1024 * 1024;
/// 拷贝整组文件的最长等待。超过即视为失败（后台复制线程会在下一个检查点自行清理
/// 半成品）；不设更长是因为 `scan_all` 会 join 本线程，卡太久会拖慢整轮刷新。
const COPY_TIMEOUT: Duration = Duration::from_secs(120);
/// WAL 数据库的伴生文件；存在才复制（回滚日志模式的库没有它们）。
///
/// **故意不复制 `-shm`**：它是 WAL 的索引，绑定了"哪台机器、哪个进程、哪个修改
/// 计数器"，从 9p 上拷来的副本索引与副本自身对不上。留着它，SQLite 会按索引认为
/// "WAL 里没有新帧"，于是只读主库、把 0 行当成真实结果——实测 Hermes 的 261 个
/// 会话全在 WAL 里，读出来却是空台账。少了 `-shm`，SQLite 会从 `-wal` 重建索引，
/// 这才是副本该有的语义。
const SIDECARS: [&str; 1] = ["-wal"];
/// SQLite 主库文件头前 16 字节固定为 `b"SQLite format 3\0"`。
const SQLITE_MAGIC: [u8; 16] = *b"SQLite format 3\0";

/// 临时副本目录守卫。**字段顺序很重要**：`ReadOnlyDb` 里 `conn` 声明在前，Drop 时
/// 先关连接（Windows 上 SQLite 打开的文件句柄随连接释放），再由本守卫删目录。
struct TempGuard {
    dir: PathBuf,
}

impl Drop for TempGuard {
    fn drop(&mut self) {
        // best-effort：删不掉（例如连接旁的其他进程还开着）也不影响本轮结果。
        let _ = std::fs::remove_dir_all(&self.dir);
    }
}

/// 一个只读数据库连接；若走的是临时副本回退，还持有副本目录（Drop 时自动清理）。
///
/// 调用方按普通 `Connection` 用：实现了 `Deref<Target = Connection>`，
/// `&ReadOnlyDb` 可以直接传给收 `&Connection` 的函数（deref coercion）。
pub struct ReadOnlyDb {
    conn: Connection,
    /// `Some` 表示这是临时副本；连接 drop 之后守卫才清目录。
    _temp: Option<TempGuard>,
}

impl std::ops::Deref for ReadOnlyDb {
    type Target = Connection;

    fn deref(&self) -> &Connection {
        &self.conn
    }
}

/// 以只读方式打开一个本机 AI 工具的 SQLite 库。
///
/// 先直接只读打开（与 [`super::open_sqlite_readonly`] 相同的语义/flags，含首次
/// `SELECT count(*) FROM sqlite_master` 验证）；失败则回退到「复制到临时目录再
/// 打开副本」。两条路都失败返回 `None`，由调用方跳过本轮。
///
/// 0 字节文件直接判失败：SQLite 会把空文件当"合法空库"接受并让 `sqlite_master`
/// 查询成功，那会把刚创建/被截断的残骸显示成"读到 0 行"，与"打不开"无法区分，
/// 也必然没有用量数据可读。
pub fn open_readonly(db_path: &Path) -> Option<ReadOnlyDb> {
    let meta = std::fs::metadata(db_path).ok()?;
    if !meta.is_file() || meta.len() == 0 {
        return None;
    }

    if let Some(conn) = super::open_sqlite_readonly(db_path) {
        return Some(ReadOnlyDb { conn, _temp: None });
    }

    // 直接打开失败：可能是 9p/WAL 的 shm 锁协商失败（busy），也可能只是普通的
    // 长锁；两种都用同一套副本回退。副本内部先做 WAL 恢复再以只读语义使用。
    let db = copy_to_temp_and_open(db_path)?;
    tracing::info!(
        target: "tum.local",
        path = %db_path.display(),
        "直接只读打开失败，已改用临时副本读取"
    );
    Some(db)
}

/// 回退路径本体：校验文件头 → 体积检查 → 后台线程拷贝主库 + `-wal` → 打开副本。
///
/// 单独拆出来是为了能直接单测「拷贝 + 打开副本」的组合（构造"直接打开必然失败"
/// 的场景依赖平台行为，不可靠）。
fn copy_to_temp_and_open(db_path: &Path) -> Option<ReadOnlyDb> {
    // 非 SQLite 文件没有复制价值（可能几十 MB 的垃圾），先看文件头再动手。
    if !has_sqlite_header(db_path) {
        return None;
    }
    let total = file_set_size(db_path)?;
    if total > MAX_COPY_BYTES {
        tracing::warn!(
            target: "tum.local",
            path = %db_path.display(),
            bytes = total,
            "SQLite 库超过回退复制上限，本轮跳过该数据源"
        );
        return None;
    }

    let cancel = Arc::new(AtomicBool::new(false));
    let (tx, rx) = std::sync::mpsc::channel();
    let src = db_path.to_path_buf();
    let worker_cancel = Arc::clone(&cancel);
    std::thread::Builder::new()
        .name("sqlite-ro-copy".to_string())
        .spawn(move || {
            // 接收端可能已超时放弃，send 失败无妨。
            let _ = tx.send(copy_to_unique_temp_dir(&src, &worker_cancel));
        })
        .ok()?;

    let dir = match rx.recv_timeout(COPY_TIMEOUT) {
        Ok(Some(dir)) => dir,
        Ok(None) => {
            tracing::warn!(
                target: "tum.local",
                path = %db_path.display(),
                "临时副本拷贝失败，本轮跳过该数据源"
            );
            return None;
        }
        Err(_) => {
            // 超时：置取消位让复制线程在下一个检查点删掉半成品；再兜底收一次它
            // 可能已经发出（但错过截止时间）的成功结果，避免临时目录泄漏。
            cancel.store(true, Ordering::Relaxed);
            std::thread::spawn(move || {
                if let Ok(Some(dir)) = rx.recv() {
                    let _ = std::fs::remove_dir_all(dir);
                }
            });
            tracing::warn!(
                target: "tum.local",
                path = %db_path.display(),
                timeout_ms = COPY_TIMEOUT.as_millis() as u64,
                "临时副本拷贝超时，本轮跳过该数据源"
            );
            return None;
        }
    };

    // 副本里保留了原文件名（`state.db` 及其 `-wal`/`-shm`），SQLite 会自动配对。
    let Some(name) = db_path.file_name() else {
        let _ = std::fs::remove_dir_all(&dir);
        return None;
    };
    match open_copy(&dir.join(name)) {
        Some(conn) => Some(ReadOnlyDb {
            conn,
            _temp: Some(TempGuard { dir }),
        }),
        None => {
            let _ = std::fs::remove_dir_all(&dir);
            tracing::warn!(
                target: "tum.local",
                path = %db_path.display(),
                "临时副本也只读打开失败，本轮跳过该数据源"
            );
            None
        }
    }
}

/// 读文件头判断是不是 SQLite 主库。读不到/不够 16 字节都算不是。
fn has_sqlite_header(db_path: &Path) -> bool {
    use std::io::Read;

    let Ok(mut file) = std::fs::File::open(db_path) else {
        return false;
    };
    let mut head = [0u8; 16];
    file.read_exact(&mut head).is_ok() && head == SQLITE_MAGIC
}

/// 在副本上打开并首查验证。
///
/// 副本是我们自己的私有临时文件（原库只被读过一次、一个字节都没改），所以这里
/// 用**读写**方式打开：SQLite 只有能写时才会做 WAL 恢复（把 `-wal` 里的帧并回主库
/// 并重建 `-shm` 索引），只读连接遇到"需要恢复"的库会直接失败或读到空表。恢复完成
/// 后立刻 `PRAGMA query_only = 1`，对上层仍是只读语义。
fn open_copy(db_path: &Path) -> Option<Connection> {
    use rusqlite::OpenFlags;

    let flags = OpenFlags::SQLITE_OPEN_READ_WRITE | OpenFlags::SQLITE_OPEN_URI;
    let conn = Connection::open_with_flags(db_path, flags).ok()?;
    let _ = conn.busy_timeout(Duration::from_millis(500));
    // WAL 恢复：把 `-wal` 的帧 checkpoint 进主库并重建索引。失败也不致命（可能是
    // 回滚日志模式，或 WAL 已被上一步打开时的隐式恢复处理掉）。
    let _ = conn.execute_batch("PRAGMA wal_checkpoint(TRUNCATE);");
    let _ = conn.execute_batch("PRAGMA query_only = 1;");
    conn.query_row("SELECT count(*) FROM sqlite_master", [], |r| {
        r.get::<_, i64>(0)
    })
    .ok()?;
    Some(conn)
}

/// 主库 + 存在的 `-wal` 组成的待复制文件组（主库在前，WAL 在后）。
fn file_set(db_path: &Path) -> Vec<PathBuf> {
    let mut paths = vec![db_path.to_path_buf()];
    for suffix in SIDECARS {
        let sidecar = sidecar_path(db_path, suffix);
        if sidecar.is_file() {
            paths.push(sidecar);
        }
    }
    paths
}

/// 拼出伴生文件路径（`state.db` + `-wal` → `state.db-wal`），不经过字符串格式化，
/// 避免非 UTF-8 路径被破坏。
fn sidecar_path(db_path: &Path, suffix: &str) -> PathBuf {
    let mut raw = db_path.as_os_str().to_os_string();
    raw.push(suffix);
    PathBuf::from(raw)
}

/// 整组文件的体积合计；任何一个文件读不到元数据都算失败（`None`）。
fn file_set_size(db_path: &Path) -> Option<u64> {
    let mut total = 0u64;
    for path in file_set(db_path) {
        total = total.saturating_add(std::fs::metadata(&path).ok()?.len());
    }
    Some(total)
}

/// 创建一个本次进程内唯一的临时目录（`temp_dir` + pid + 自增序号）。
fn unique_temp_dir() -> Option<PathBuf> {
    use std::sync::atomic::AtomicU64;

    static SEQ: AtomicU64 = AtomicU64::new(0);
    for _ in 0..64 {
        let seq = SEQ.fetch_add(1, Ordering::Relaxed);
        let dir = std::env::temp_dir().join(format!("tum-sqlite-ro-{}-{}", std::process::id(), seq));
        match std::fs::create_dir(&dir) {
            Ok(()) => return Some(dir),
            // 目录已被占用（上一轮同名残留）就换下一个序号。
            Err(e) if e.kind() == std::io::ErrorKind::AlreadyExists => continue,
            Err(_) => return None,
        }
    }
    None
}

/// 把主库 + `-wal` 复制进一个新的唯一临时目录；失败时删掉半成品目录再返回 `None`。
fn copy_to_unique_temp_dir(src_db: &Path, cancel: &AtomicBool) -> Option<PathBuf> {
    let dir = unique_temp_dir()?;
    if !copy_file_set(src_db, &dir, cancel) {
        let _ = std::fs::remove_dir_all(&dir);
        return None;
    }
    Some(dir)
}

/// 逐个复制文件组；每个文件前检查取消位，结束后再查一次（超时后不留半成品）。
fn copy_file_set(src_db: &Path, dst_dir: &Path, cancel: &AtomicBool) -> bool {
    for src in file_set(src_db) {
        if cancel.load(Ordering::Relaxed) {
            return false;
        }
        let Some(name) = src.file_name() else {
            return false;
        };
        if std::fs::copy(&src, dst_dir.join(name)).is_err() {
            return false;
        }
    }
    !cancel.load(Ordering::Relaxed)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// 现场探针（需要真实 WSL 库，默认忽略）：把每一步的真实错误打出来。
    ///
    /// 用法：`cargo test --lib -- --ignored --nocapture probe_real_wsl_hermes`
    #[test]
    #[ignore]
    fn probe_real_wsl_hermes() {
        // 路径从环境变量读：仓库里不写死任何人的机器路径。
        let Ok(raw) = std::env::var("TUM_PROBE_DB") else {
            println!("TUM_PROBE_DB 未设置，跳过");
            return;
        };
        let path = std::path::Path::new(&raw);
        println!("exists={}", path.exists());
        match temp_probe_direct(path) {
            Ok(()) => println!("direct RO ok"),
            Err(e) => println!("direct RO err: {e}"),
        }
        match copy_to_temp_and_open(path) {
            Some(db) => {
                let n: rusqlite::Result<i64> =
                    db.query_row("SELECT count(*) FROM sessions", [], |r| r.get(0));
                println!("copy open ok; sessions={:?}", n);
                let m: rusqlite::Result<i64> =
                    db.query_row("SELECT count(*) FROM session_model_usage", [], |r| r.get(0));
                println!("copy open ok; smu={:?}", m);
            }
            None => println!("copy open FAILED"),
        }
    }

    fn temp_probe_direct(path: &Path) -> rusqlite::Result<()> {
        use rusqlite::OpenFlags;
        let flags = OpenFlags::SQLITE_OPEN_READ_ONLY | OpenFlags::SQLITE_OPEN_URI;
        let conn = Connection::open_with_flags(path, flags)?;
        conn.query_row("SELECT count(*) FROM sqlite_master", [], |r| r.get::<_, i64>(0))?;
        let n: i64 = conn.query_row("SELECT count(*) FROM sessions", [], |r| r.get(0))?;
        println!("direct RO sessions={n}");
        Ok(())
    }
    /// 测试用根目录（每次先清空，避免上次失败留下的残骸污染断言）。
    fn temp_root(tag: &str) -> PathBuf {
        let root = std::env::temp_dir().join(format!(
            "tum-sqlite-ro-test-{}-{}",
            tag,
            std::process::id()
        ));
        let _ = std::fs::remove_dir_all(&root);
        std::fs::create_dir_all(&root).expect("create test root");
        root
    }

    /// 建一个 WAL 模式的库并写入一行；返回仍打开的写连接（保持 `-wal`/`-shm` 存在，
    /// 这正是 Hermes 常驻 gateway 时的真实形态）。
    fn wal_db_with_row(db_path: &Path) -> Connection {
        let writer = Connection::open(db_path).expect("open writer");
        writer
            .pragma_update(None, "journal_mode", "wal")
            .expect("switch to WAL");
        writer
            .execute_batch(
                "CREATE TABLE sessions(id TEXT PRIMARY KEY, tokens INTEGER NOT NULL);
                 INSERT INTO sessions(id, tokens) VALUES ('a', 10);",
            )
            .expect("seed rows");
        writer
    }

    /// 正常 WAL 库（写者在线、`-wal` 未 checkpoint）走**直接只读打开**就能读到数据，
    /// 不该触发副本回退。
    #[test]
    fn opens_wal_database_and_reads_rows_directly() {
        let root = temp_root("wal");
        let db_path = root.join("state.db");
        let writer = wal_db_with_row(&db_path);
        assert!(
            sidecar_path(&db_path, "-wal").is_file(),
            "前提：写者在线时应存在 -wal"
        );

        let db = open_readonly(&db_path).expect("WAL 库应能只读打开");
        assert!(db._temp.is_none(), "正常库不应走临时副本回退");
        let rows: i64 = db
            .query_row("SELECT count(*) FROM sessions", [], |r| r.get(0))
            .expect("read rows");
        assert_eq!(rows, 1);

        drop(db);
        drop(writer);
        let _ = std::fs::remove_dir_all(&root);
    }

    /// 回退用 helper 的端到端测试：写者在线、数据还在 `-wal` 里（未 checkpoint）时，
    /// 拷出来的副本必须带着 WAL 内容，读到的是最新行而不是空库。
    #[test]
    fn copy_fallback_reads_uncheckpointed_wal_rows() {
        let root = temp_root("walcopy");
        let db_path = root.join("state.db");
        let writer = wal_db_with_row(&db_path);

        let db = copy_to_temp_and_open(&db_path).expect("副本回退应能打开");
        assert!(db._temp.is_some(), "应走临时副本");
        let rows: i64 = db
            .query_row("SELECT count(*) FROM sessions", [], |r| r.get(0))
            .expect("读到副本里的行");
        assert_eq!(rows, 1);

        drop(db); // Drop 后副本目录应被清掉
        drop(writer);
        let _ = std::fs::remove_dir_all(&root);
    }

    /// 拷来的 `-shm`（WAL 索引）与副本对不上时，绝不能当成"WAL 是空的"。
    ///
    /// 真实事故形态：Hermes 的 `state.db` 在 9p 上，`-wal` 里躺着全部 261 个会话，
    /// 拷 `-shm` 过去后 SQLite 按旧索引认为没有新帧，`sessions` 查出 0 行——扫描器
    /// 拿到一份"合法但为空"的结果，账本自 10-08 起静默断流。修法是不拷 `-shm`，
    /// 让 SQLite 从 `-wal` 重建索引。本测试把源码目录里的 `-shm` 写成垃圾来钉住它。
    #[test]
    fn stale_shm_in_the_source_does_not_hide_wal_rows() {
        let root = temp_root("staleshm");
        let writer = wal_db_with_row(&root.join("state.db"));
        let wal_rows: i64 = writer
            .query_row("SELECT count(*) FROM sessions", [], |r| r.get(0))
            .expect("writer sees its rows");
        assert_eq!(wal_rows, 1, "前提：行在库可见");

        // 复制主库 + -wal 到"带垃圾 -shm 的源目录"，模拟 9p 上跨机器拷来的索引。
        let src = root.join("src");
        std::fs::create_dir_all(&src).expect("src dir");
        std::fs::copy(root.join("state.db"), src.join("state.db")).expect("copy db");
        std::fs::copy(
            sidecar_path(&root.join("state.db"), "-wal"),
            src.join("state.db-wal"),
        )
        .expect("copy wal");
        std::fs::write(sidecar_path(&src.join("state.db"), "-shm"), b"not a wal index")
            .expect("write junk shm");

        let db = copy_to_temp_and_open(&src.join("state.db")).expect("副本应能打开");
        let rows: i64 = db
            .query_row("SELECT count(*) FROM sessions", [], |r| r.get(0))
            .expect("读到副本里的行");
        assert_eq!(rows, 1, "旧的 -shm 不得让副本把 WAL 里的行读丢");

        drop(db);
        drop(writer);
        let _ = std::fs::remove_dir_all(&root);
    }

    /// 构造"直接只读打开必然失败"的场景：把 `-shm` 造成一个目录。SQLite 映射 shm
    /// 必然失败，而回退只复制是普通文件的伴生文件，到下个临时目录即可正常打开。
    #[test]
    fn falls_back_to_temp_copy_when_direct_open_fails() {
        let root = temp_root("fallback");
        let db_path = root.join("state.db");
        // 写者 drop 时 SQLite 会 checkpoint 并删掉 -wal/-shm，数据落进主库。
        drop(wal_db_with_row(&db_path));
        std::fs::create_dir(sidecar_path(&db_path, "-shm")).expect("create -shm as dir");

        assert!(
            super::super::open_sqlite_readonly(&db_path).is_none(),
            "前提：-shm 是目录时直接只读打开应失败"
        );

        let db = open_readonly(&db_path).expect("副本回退应能打开");
        assert!(db._temp.is_some(), "应走临时副本");
        let rows: i64 = db
            .query_row("SELECT count(*) FROM sessions", [], |r| r.get(0))
            .expect("读到副本里的行");
        assert_eq!(rows, 1);
        let dir = db._temp.as_ref().expect("temp dir").dir.clone();
        drop(db);
        assert!(!dir.exists(), "连接 drop 后临时副本目录应被清理");

        let _ = std::fs::remove_dir_all(&root);
    }

    /// 不存在的路径 → None（不 panic、不尝试创建）。
    #[test]
    fn missing_path_returns_none() {
        let root = temp_root("missing");
        assert!(open_readonly(&root.join("nope.db")).is_none());
        let _ = std::fs::remove_dir_all(&root);
    }

    /// 不是 SQLite 的文件 → None（直接打开首查失败，副本回退也被文件头拦下）。
    #[test]
    fn non_sqlite_file_returns_none() {
        let root = temp_root("notdb");
        let path = root.join("state.db");
        std::fs::write(&path, b"definitely not a sqlite database, just text").unwrap();
        assert!(open_readonly(&path).is_none());
        let _ = std::fs::remove_dir_all(&root);
    }

    /// 0 字节文件 → None（SQLite 会把它当空库接受，这里必须主动判失败）。
    #[test]
    fn empty_file_returns_none() {
        let root = temp_root("empty");
        let path = root.join("state.db");
        std::fs::write(&path, b"").unwrap();
        assert!(open_readonly(&path).is_none());
        let _ = std::fs::remove_dir_all(&root);
    }
}
