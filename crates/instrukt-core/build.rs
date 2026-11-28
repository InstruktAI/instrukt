use std::io::{self, Write as _};
use std::process::ExitCode;

fn main() -> ExitCode {
    let mut out = io::stdout().lock();
    let _ = writeln!(out, "cargo::rustc-check-cfg=cfg(instrukt_release_build)");
    let _ = writeln!(out, "cargo::rerun-if-env-changed=INSTRUKT_RELEASE_BUILD");

    if std::env::var_os("INSTRUKT_RELEASE_BUILD").as_deref() == Some(std::ffi::OsStr::new("1")) {
        let profile = std::env::var("PROFILE").unwrap_or_default();
        if profile != "release" {
            let _ = writeln!(
                io::stderr(),
                "error: INSTRUKT_RELEASE_BUILD=1 requires the release profile (got PROFILE={profile:?})"
            );
            return ExitCode::FAILURE;
        }
        let _ = writeln!(out, "cargo::rustc-cfg=instrukt_release_build");
    }
    ExitCode::SUCCESS
}
