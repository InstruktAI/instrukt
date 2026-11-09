#![allow(clippy::unwrap_used, clippy::expect_used, reason = "test setup")]
use crate::common;
use common::TestEnv;
use std::os::unix::fs::PermissionsExt;
use std::path::{Path, PathBuf};

fn script(file: &Path, text: &str) {
    std::fs::write(file, text).unwrap();
    std::fs::set_permissions(file, std::fs::Permissions::from_mode(0o755)).unwrap();
}
struct Release {
    env: TestEnv,
    bin: PathBuf,
}
impl Release {
    fn new() -> Self {
        let mut env = TestEnv::new();
        // Publication uses sha256sum on both platforms; macOS ships shasum.
        script(
            &env.path("runtime-bin/sha256sum"),
            "#!/bin/sh\nexec shasum -a 256 \"$@\"\n",
        );
        let bin = env.path("bin");
        std::fs::create_dir_all(&bin).unwrap();
        script(
            &bin.join("uname"),
            "#!/bin/sh\ncase $1 in -s) echo Linux;; -m) echo x86_64;; esac\n",
        );
        script(
            &bin.join("curl"),
            &format!(
                "#!/bin/sh\ncase $* in *releases/latest*) echo https://github.com/InstruktAI/instrukt/releases/tag/v0.1.0; exit;; esac\nfor a in \"$@\"; do url=$a; done\ncat '{}/assets/'\"${{url##*/}}\"\n",
                env.path("").display()
            ),
        );
        std::fs::create_dir_all(env.path("assets")).unwrap();
        let path = format!("{}:{}", bin.display(), env.injected("PATH").unwrap());
        env.set_env("PATH", &path);
        Self { env, bin }
    }
    fn asset(&self, version: &str, reported: &str) {
        let name = format!("instrukt-v{version}-x86_64-unknown-linux-gnu");
        let folder = self.env.path(&format!("payload/{name}"));
        std::fs::create_dir_all(&folder).unwrap();
        script(
            &folder.join("instrukt"),
            &format!("#!/bin/sh\necho 'instrukt {reported}'\n"),
        );
        std::fs::write(folder.join("README.md"), "release\n").unwrap();
        let archive = self.env.path(&format!("assets/{name}.tar.gz"));
        assert!(
            std::process::Command::new("tar")
                .args(["-czf"])
                .arg(&archive)
                .arg("-C")
                .arg(folder.parent().unwrap())
                .arg(&name)
                .status()
                .unwrap()
                .success()
        );
        use sha2::{Digest, Sha256};
        let digest = hex::encode(Sha256::digest(std::fs::read(archive).unwrap()));
        std::fs::write(
            self.env.path("assets/SHA256SUMS"),
            format!("{digest}  {name}.tar.gz\n"),
        )
        .unwrap();
    }
    fn install(&self, args: &[&str]) -> std::process::Output {
        self.env
            .command(Path::new("/bin/sh"), self.env.home())
            .arg(Path::new(env!("CARGO_MANIFEST_DIR")).join("../../scripts/install.sh"))
            .args(args)
            .output()
            .unwrap()
    }
    fn installed(&self) -> PathBuf {
        self.env.home().join(".local/bin/instrukt")
    }
}
// spec: project/instrukt/spec/feature/distribution/release-installer UC-RI1
#[test]
fn installs_verified_latest_binary_and_reports_version_path_and_skill() {
    let release = Release::new();
    release.asset("0.1.0", "0.1.0");
    let output = release.install(&[]);
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    assert!(release.installed().is_file());
    insta::assert_snapshot!(
        String::from_utf8_lossy(&output.stdout)
            .replace(release.env.home().to_str().unwrap(), "[HOME]")
    );
}
// spec: project/instrukt/spec/feature/distribution/release-installer UC-RI2
#[test]
fn upgrades_pins_and_leaves_matching_binary_untouched() {
    let release = Release::new();
    release.asset("0.0.9", "0.0.9");
    assert!(release.install(&["--version", "0.0.9"]).status.success());
    let version = || {
        release
            .env
            .command(&release.installed(), release.env.home())
            .arg("--version")
            .output()
            .unwrap()
    };
    insta::assert_snapshot!(String::from_utf8_lossy(&version().stdout), @"instrukt 0.0.9");
    release.asset("0.1.0", "0.1.0");
    assert!(release.install(&[]).status.success());
    insta::assert_snapshot!(String::from_utf8_lossy(&version().stdout), @"instrukt 0.1.0");
    let before = std::fs::metadata(release.installed())
        .unwrap()
        .modified()
        .unwrap();
    assert!(release.install(&[]).status.success());
    assert_eq!(
        before,
        std::fs::metadata(release.installed())
            .unwrap()
            .modified()
            .unwrap()
    );
    release.asset("0.0.9", "0.0.9");
    assert!(release.install(&["--version", "0.0.9"]).status.success());
    insta::assert_snapshot!(String::from_utf8_lossy(&version().stdout), @"instrukt 0.0.9");
}
// spec: project/instrukt/spec/feature/distribution/release-installer UC-RI3
#[test]
fn checksum_and_binary_version_failures_preserve_old_install() {
    let release = Release::new();
    release.asset("0.1.0", "0.1.0");
    assert!(release.install(&[]).status.success());
    let old = std::fs::read(release.installed()).unwrap();
    release.asset("0.2.0", "0.2.0");
    std::fs::write(
        release.env.path("assets/SHA256SUMS"),
        "bad  instrukt-v0.2.0-x86_64-unknown-linux-gnu.tar.gz\n",
    )
    .unwrap();
    assert!(!release.install(&["--version", "0.2.0"]).status.success());
    assert_eq!(old, std::fs::read(release.installed()).unwrap());
    release.asset("0.2.0", "9.9.9");
    assert!(!release.install(&["--version", "0.2.0"]).status.success());
    assert_eq!(old, std::fs::read(release.installed()).unwrap());
}
// spec: project/instrukt/spec/feature/distribution/release-installer UC-RI4
#[test]
fn unsupported_platform_refuses_before_download_or_write() {
    let release = Release::new();
    script(&release.bin.join("uname"), "#!/bin/sh\necho unsupported\n");
    script(
        &release.bin.join("curl"),
        &format!(
            "#!/bin/sh\ntouch '{}'\nexit 77\n",
            release.env.path("download-attempted").display()
        ),
    );
    let untouched = common::LiveGuard::over(
        vec![],
        vec![release.env.home().into(), release.env.path("tmp")],
    );
    let output = release.install(&[]);
    assert!(!output.status.success());
    assert!(!release.env.path("download-attempted").exists());
    assert!(untouched.changed().is_empty());
    insta::assert_snapshot!(String::from_utf8_lossy(&output.stderr));
    assert!(!release.installed().exists());
    assert_eq!(
        std::fs::read_dir(release.env.path("tmp")).unwrap().count(),
        0
    );
}

// Smoke/wiring: validated release assets reach gh in draft-then-publish order.
#[test]
fn standalone_publication_checks_the_tagged_asset_set_before_gh_and_publishes_a_draft_first() {
    let release = Release::new();
    let version = env!("CARGO_PKG_VERSION");
    let tag = format!("v{version}");
    std::fs::write(
        release.env.home().join("Cargo.toml"),
        format!("[workspace.package]\nversion = \"{version}\"\n"),
    )
    .unwrap();
    std::fs::create_dir_all(release.env.home().join("skills/instrukt")).unwrap();
    std::fs::write(
        release.env.home().join("skills/instrukt/SKILL.md"),
        format!("metadata:\n  version: {version}\n"),
    )
    .unwrap();
    let calls = release.env.path("gh-calls");
    script(
        &release.bin.join("gh"),
        "#!/bin/sh\nprintf '%s\\n' \"$@\" >> \"$GH_CALLS\"\n",
    );
    let script = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../scripts/publish-release.sh");
    let publish = |tag: &str| {
        release
            .env
            .command(Path::new("/bin/sh"), release.env.home())
            .arg(&script)
            .env("GITHUB_REF_NAME", tag)
            .env("GITHUB_REPOSITORY", "fixture/project")
            .env("GH_CALLS", &calls)
            .output()
            .unwrap()
    };
    assert!(!publish(&tag).status.success());
    assert!(!calls.exists());
    for target in [
        "aarch64-apple-darwin",
        "x86_64-unknown-linux-gnu",
        "aarch64-unknown-linux-gnu",
    ] {
        std::fs::write(
            release
                .env
                .home()
                .join(format!("instrukt-{tag}-{target}.tar.gz")),
            target,
        )
        .unwrap();
    }
    assert!(!publish("v99.0.0").status.success());
    assert!(!calls.exists());
    let output = publish(&tag);
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    assert!(release.env.home().join("SHA256SUMS").is_file());
    let calls = std::fs::read_to_string(calls).unwrap().replace(
        script
            .parent()
            .unwrap()
            .canonicalize()
            .unwrap()
            .to_str()
            .unwrap(),
        "[SCRIPTS]",
    );
    insta::assert_snapshot!(calls);
}
