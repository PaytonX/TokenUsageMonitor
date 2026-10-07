use std::env;
use std::path::PathBuf;
use std::process::Command;

const TEST_MANIFEST: &str = r#"<?xml version="1.0" encoding="UTF-8" standalone="yes"?>
<assembly xmlns="urn:schemas-microsoft-com:asm.v1" manifestVersion="1.0">
  <dependency>
    <dependentAssembly>
      <assemblyIdentity type="win32" name="Microsoft.Windows.Common-Controls" version="6.0.0.0" processorArchitecture="*" publicKeyToken="6595b64144ccf1df" language="*"/>
    </dependentAssembly>
  </dependency>
</assembly>
"#;

const RESOURCE_LIB_NAME: &str = "cargo_test_manifest_res";

fn main() {
    tauri_build::build();

    if env::var("CARGO_CFG_TARGET_OS").as_deref() == Ok("windows") {
        match env::var("CARGO_CFG_TARGET_ENV").as_deref() {
            Ok("msvc") => delayload_comctl32(),
            Ok("gnu") => embed_gnu_test_manifest(),
            _ => {}
        }
    }
}

/// MSVC 目标的测试二进制缺 Common-Controls v6 manifest 的兜底。
///
/// tauri-build 只把 manifest 嵌进主程序（cargo:rustc-link-arg-bins），
/// `cargo test` 生成的测试 harness 走的是另一条编译路径，拿不到这份资源。
/// 缺了 `<dependency>` 节点时，Windows 加载器按普通搜索顺序把 `comctl32.dll`
/// 解析到 System32 里的 v5.82，而不是 WinSxS 的 v6.x。v5.82 不导出
/// `TaskDialogIndirect` / `SetWindowSubclass`，于是测试二进制在进入 `main`
/// 之前就被拒绝启动：
///
///     error: test failed, to rerun pass `--test router_proxy`
///     (exit code: 0xc0000139, STATUS_ENTRYPOINT_NOT_FOUND)
///
/// 与其像 GNU 分支那样另找一套资源编译工具链（依赖外部 windres/ar，CI 上并不
/// 存在），MSVC 侧改用链接器原生的延迟导入：把 comctl32 从硬导入转为延迟导入，
/// 链接器只为它生成一个存桩，首次真正调用该 DLL 的导出时才做符号解析。测试
/// harness 从不触碰这些 GUI 入口点，于是失败不再发生；主程序本身带
/// tauri-build 嵌入的 manifest，真要调用时仍走 Fusion 拿到 v6.x。
fn delayload_comctl32() {
    println!("cargo:rustc-link-arg=/DELAYLOAD:comctl32.dll");
    println!("cargo:rustc-link-lib=delayimp");
}

fn out_dir() -> PathBuf {
    PathBuf::from(env::var("OUT_DIR").expect("OUT_DIR not set"))
}

fn find_tool(names: &[&str]) -> Option<PathBuf> {
    let paths = env::var_os("PATH")?;
    for dir in env::split_paths(&paths) {
        for name in names {
            let candidate = dir.join(name);
            if candidate.is_file() {
                return Some(candidate);
            }
        }
    }
    None
}

fn embed_gnu_test_manifest() {
    let dir = out_dir();
    let manifest = dir.join("cargo-test.manifest");
    if let Err(err) = std::fs::write(&manifest, TEST_MANIFEST) {
        println!("cargo:warning=failed to write cargo-test.manifest: {err}");
        return;
    }
    let Some(windres) = find_tool(&["x86_64-w64-mingw32-windres.exe", "windres.exe"]) else {
        println!("cargo:warning=windres not found on PATH; unit-test binaries will lack the Common-Controls v6 manifest (TaskDialogIndirect entry-point error on Windows)");
        return;
    };
    let Some(ar) = find_tool(&["llvm-ar.exe", "x86_64-w64-mingw32-ar.exe", "ar.exe"]) else {
        println!("cargo:warning=llvm-ar not found on PATH; unit-test binaries will lack the Common-Controls v6 manifest");
        return;
    };
    let manifest_fwd = manifest.to_string_lossy().replace('\\', "/");
    let rc = dir.join("cargo-test.rc");
    if let Err(err) = std::fs::write(&rc, format!("1 24 \"{manifest_fwd}\"\n")) {
        println!("cargo:warning=failed to write cargo-test.rc: {err}");
        return;
    }
    let obj = dir.join("cargo-test-resource.o");
    let compiled = Command::new(&windres)
        .arg(&rc)
        .args(["-O", "coff"])
        .arg(&obj)
        .status()
        .map(|status| status.success())
        .unwrap_or(false);
    if !compiled {
        println!("cargo:warning=windres failed; unit-test binaries will lack the Common-Controls v6 manifest");
        return;
    }
    let archive = dir.join(format!("lib{RESOURCE_LIB_NAME}.a"));
    let archived = Command::new(&ar)
        .args(["rcs"])
        .arg(&archive)
        .arg(&obj)
        .status()
        .map(|status| status.success())
        .unwrap_or(false);
    if !archived {
        println!("cargo:warning=failed to archive manifest resource; unit-test binaries will lack the Common-Controls v6 manifest");
        return;
    }
    println!("cargo:rustc-link-search=native={}", dir.display());
}