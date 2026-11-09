//! Provision an executable without starting a distribution-owned Redis service.
use super::Request;
use crate::process::Invocation;
use std::{path::PathBuf, time::Duration};
fn on_path(name: &str, path_env: &str) -> Option<PathBuf> {
    std::env::split_paths(path_env)
        .map(|d| d.join(name))
        .find(|p| p.is_file())
}
pub(super) fn server(request: &Request<'_>) -> anyhow::Result<PathBuf> {
    if let Some(executable) = on_path("redis-server", request.wake.path_env) {
        return Ok(executable);
    }
    let packages = request.home.state_dir().join("redis-package");
    let extracted = packages.join("root/usr/bin/redis-server");
    if extracted.is_file() {
        return Ok(extracted);
    }
    anyhow::ensure!(
        !request.check,
        "redis-server is absent; run instrukt install with a supported package provisioner"
    );
    if cfg!(target_os = "macos") {
        anyhow::ensure!(
            on_path("brew", request.wake.path_env).is_some(),
            "Redis requires Homebrew or an operator-supplied executable; install that prerequisite, then rerun instrukt install"
        );
        let done =
            Invocation::program("brew", &["install", "redis"]).run(Duration::from_secs(300))?;
        anyhow::ensure!(
            done.success,
            "Redis package prerequisite failed; repair Homebrew without changing foreign services, then rerun instrukt install"
        );
        return on_path("redis-server", request.wake.path_env)
            .ok_or_else(|| anyhow::anyhow!("redis-server is not on PATH after provisioning"));
    }
    anyhow::ensure!(
        on_path("apt-get", request.wake.path_env).is_some()
            && on_path("dpkg-deb", request.wake.path_env).is_some(),
        "no supported Redis provisioner: provide redis-server with foreign service state preserved, then rerun instrukt install"
    );
    // Download/extract only: no dpkg install, maintainer scripts or foreign services.
    // A missing shared library remains a prerequisite when the executable is checked.
    std::fs::create_dir_all(&packages)?;
    let done = Invocation::program("apt-get", &["download", "redis-server"])
        .cwd(&packages)
        .run(Duration::from_secs(300))?;
    anyhow::ensure!(
        done.success,
        "Redis package download failed; restore apt repository access and rerun instrukt install"
    );
    let mut archives = std::fs::read_dir(&packages)?
        .map(|e| e.map(|e| e.path()))
        .collect::<Result<Vec<_>, _>>()?;
    archives.retain(|p| p.extension().is_some_and(|e| e == "deb"));
    anyhow::ensure!(
        archives.len() == 1,
        "Redis package cache is ambiguous; move obsolete .deb files out of {} and rerun instrukt install",
        packages.display()
    );
    let done = Invocation::program(
        "dpkg-deb",
        &[
            "--extract",
            &archives[0].display().to_string(),
            &packages.join("root").display().to_string(),
        ],
    )
    .run(Duration::from_secs(60))?;
    anyhow::ensure!(
        done.success && extracted.is_file(),
        "Redis package extraction failed; provide a working redis-server and rerun instrukt install"
    );
    Ok(extracted)
}
