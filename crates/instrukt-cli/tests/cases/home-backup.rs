//! Real-binary CLI integration scenarios for `project/instrukt/spec/feature/runner/home-backup`.

use std::collections::{BTreeMap, BTreeSet};
use std::io::{Read as _, Write as _};
use std::os::unix::fs::PermissionsExt as _;
use std::path::PathBuf;

use crate::common::TestEnv;
use crate::fixtures::s3::FakeS3;
use age::secrecy::ExposeSecret as _;
use instrukt_core::config::GLOBAL_SCHEMA_VERSION;
use instrukt_core::fsx::LockGuard;
use instrukt_core::state_db::rusqlite::Connection;

struct BackupFixture {
    env: TestEnv,
    s3: FakeS3,
    identity: age::x25519::Identity,
    identity_file: PathBuf,
    notify_log: PathBuf,
}

impl BackupFixture {
    fn new(keep: u32, prefix: Option<&str>) -> anyhow::Result<Self> {
        let env = TestEnv::new();
        let s3 = FakeS3::start()?;
        let identity = age::x25519::Identity::generate();
        let recipient = identity.to_public().to_string();

        let identity_file = env.path("operator-age-identity.txt");
        std::fs::write(
            &identity_file,
            format!("{}\n", identity.to_string().expose_secret()),
        )?;
        std::fs::set_permissions(&identity_file, std::fs::Permissions::from_mode(0o600))?;

        let notify_log = env.path("notify.log");
        let notify_script = env.path("notify.sh");
        std::fs::write(
            &notify_script,
            format!(
                "#!/bin/sh\nprintf '%s\\n' \"$1\" >> {}\n",
                notify_log.display()
            ),
        )?;
        std::fs::set_permissions(&notify_script, std::fs::Permissions::from_mode(0o700))?;

        let home_dir = env.instrukt_home();
        std::fs::create_dir_all(home_dir.join("state"))?;

        let mut lines = vec![
            format!("schema_version: {GLOBAL_SCHEMA_VERSION}"),
            "runner:".to_owned(),
            "  runtime: t3".to_owned(),
            format!("  notify: {}", notify_script.display()),
            "backup:".to_owned(),
            format!("  endpoint: {}", s3.endpoint()),
            "  region: auto".to_owned(),
            "  bucket: home-backups".to_owned(),
            "  access_key_id: ${BACKUP_ACCESS_KEY_ID}".to_owned(),
            "  secret_access_key: ${BACKUP_SECRET_ACCESS_KEY}".to_owned(),
            format!("  age_recipient: {recipient}"),
            format!("  keep: {keep}"),
        ];
        if let Some(p) = prefix {
            lines.push(format!("  prefix: {p}"));
        }
        lines.push(String::new());
        std::fs::write(home_dir.join("instrukt.yaml"), lines.join("\n"))?;
        std::fs::write(
            home_dir.join("secrets.env"),
            "BACKUP_ACCESS_KEY_ID=AKIA_BACKUP_TEST_KEY\nBACKUP_SECRET_ACCESS_KEY=SUPER_SECRET_BACKUP_VALUE\n",
        )?;
        std::fs::set_permissions(
            home_dir.join("secrets.env"),
            std::fs::Permissions::from_mode(0o600),
        )?;

        Ok(Self {
            env,
            s3,
            identity,
            identity_file,
            notify_log,
        })
    }

    fn machine_id(&self) -> anyhow::Result<String> {
        Ok(
            std::fs::read_to_string(self.env.instrukt_home().join("state/machine-id"))?
                .trim()
                .to_owned(),
        )
    }
}

fn decrypt_archive_contents(
    ciphertext: &[u8],
    identity: &age::x25519::Identity,
) -> anyhow::Result<BTreeMap<String, Vec<u8>>> {
    let decryptor = age::Decryptor::new(ciphertext)?;
    let age_reader = decryptor.decrypt(std::iter::once(identity as &dyn age::Identity))?;
    let zstd_reader = zstd::stream::read::Decoder::new(age_reader)?;
    let mut tar_archive = tar::Archive::new(zstd_reader);
    let mut out = BTreeMap::new();
    for entry in tar_archive.entries()? {
        let mut entry = entry?;
        let name = entry.path()?.display().to_string();
        let mut bytes = Vec::new();
        entry.read_to_end(&mut bytes)?;
        out.insert(name, bytes);
    }
    Ok(out)
}

// spec: project/instrukt/spec/feature/runner/home-backup UC-HB1
#[test]
fn home_backup_snapshots_irreplaceable_state_encrypts_verifies_and_handles_failures() {
    let mut fx = BackupFixture::new(5, Some("hosts")).unwrap();
    fx.env.set_env("INSTRUKT_NOW", "2030-01-06T03:00:00Z");
    let home = fx.env.instrukt_home();

    // Keep an active WAL connection open with an uncheckpointed row residing
    // solely in `instrukt.db-wal` throughout `instrukt backup run`, so a raw
    // file copy of `instrukt.db` without SQLite's online backup API would miss it.
    let db_path = home.join("state/instrukt.db");
    let live_wal_conn = Connection::open(&db_path).unwrap();
    live_wal_conn
        .execute_batch(
            "PRAGMA journal_mode=WAL;
             PRAGMA wal_autocheckpoint=0;
             CREATE TABLE notes(body TEXT);
             INSERT INTO notes VALUES ('base-row');
             PRAGMA wal_checkpoint(TRUNCATE);
             INSERT INTO notes VALUES ('live-wal-row');",
        )
        .unwrap();
    let wal_meta = std::fs::metadata(home.join("state/instrukt.db-wal")).unwrap();
    assert!(wal_meta.len() > 0, "expected uncheckpointed WAL frames");
    let raw_main_db = std::fs::read(&db_path).unwrap();
    assert!(
        !raw_main_db
            .windows(b"live-wal-row".len())
            .any(|w| w == b"live-wal-row"),
        "live-wal-row must live only in the WAL sidecar before online backup"
    );
    let extra_db = home.join("state/custom.sqlite");
    {
        let conn = Connection::open(&extra_db).unwrap();
        conn.execute_batch("CREATE TABLE k(v TEXT); INSERT INTO k VALUES ('dynamic');")
            .unwrap();
    }
    std::fs::write(home.join("agents.yaml"), "profiles: {}\n").unwrap();
    std::fs::create_dir_all(home.join("state/eval-fingerprint")).unwrap();
    std::fs::write(home.join("state/eval-fingerprint/p.json"), "{}\n").unwrap();
    std::fs::write(home.join("state/agents-deployment.json"), "{}\n").unwrap();
    std::fs::create_dir_all(home.join("state/known-issues")).unwrap();
    std::fs::write(home.join("state/known-issues/demo.json"), "[]\n").unwrap();
    std::fs::write(home.join("state/known-issues/demo.lock"), "").unwrap();
    std::fs::create_dir_all(home.join("state/company-generation")).unwrap();
    std::fs::write(home.join("state/company-generation/acme.json"), "{}\n").unwrap();

    // Research notes (included) vs raw git checkout under git/ (excluded).
    std::fs::create_dir_all(home.join("git/raw-checkout/.git")).unwrap();
    std::fs::write(home.join("git/index.md"), "# index\n").unwrap();
    std::fs::write(home.join("git/raw-checkout/code.rs"), "fn main() {}\n").unwrap();
    std::fs::create_dir_all(home.join("explore")).unwrap();
    std::fs::write(home.join("explore/report.md"), "# report\n").unwrap();

    // Legacy company home (instrukt.yaml should forward-convert to company.yaml).
    let company = home.join("companies/acme");
    std::fs::create_dir_all(company.join(".instrukt/schemas")).unwrap();
    std::fs::create_dir_all(company.join(".git")).unwrap();
    std::fs::write(
        company.join("instrukt.yaml"),
        "schema_version: 5\ncompany: acme\n",
    )
    .unwrap();
    std::fs::write(company.join("secrets.env"), "COMPANY_KEY=secret\n").unwrap();
    std::fs::write(company.join(".instrukt/schemas/s.json"), "{}\n").unwrap();
    std::fs::write(company.join("tracked.md"), "git tracked\n").unwrap();
    {
        let conn = Connection::open(company.join(".instrukt/knowledge-proposals.db")).unwrap();
        conn.execute_batch("CREATE TABLE p(id INTEGER); INSERT INTO p VALUES (1);")
            .unwrap();
    }

    // Excluded caches, docs, rendered, blueprint, locks, stray recovery files.
    for (rel, data) in [
        ("cache/git/repo.bin", b"cached".as_slice()),
        ("docs/general/rule.md", b"published"),
        ("rendered/AGENTS.md", b"rendered"),
        ("state/index.yaml", b"index"),
        ("state/blueprint/p/g.json", b"graph"),
        ("state/instrukt.db.corrupt.1", b"corrupt"),
        ("locks/docs.lock", b""),
    ] {
        let p = home.join(rel);
        std::fs::create_dir_all(p.parent().unwrap()).unwrap();
        std::fs::write(p, data).unwrap();
    }

    let out = fx.env.run(fx.env.home(), ["backup", "run"]).success();
    drop(live_wal_conn);
    let stdout = String::from_utf8_lossy(&out.get_output().stdout);
    assert!(stdout.contains("published hosts/"), "{stdout}");
    assert!(
        fx.s3.wire_protocol_violations().is_empty(),
        "wire protocol violations: {:?}",
        fx.s3.wire_protocol_violations()
    );

    assert!(company.join("company.yaml").exists());
    assert!(!company.join("instrukt.yaml").exists());

    let mid = fx.machine_id().unwrap();
    let expected_key = format!("hosts/{mid}/instrukt-home.20300106T030000Z.tar.zst.age");
    let keys = fx.s3.keys();
    assert!(keys.contains(&expected_key), "{keys:?}");
    assert!(
        keys.contains(&format!("_validated/{expected_key}")),
        "{keys:?}"
    );
    assert!(
        !keys.iter().any(|k| k.starts_with("_staging/")),
        "staging should be empty after publication: {keys:?}"
    );

    let ciphertext = fx.s3.get_bytes(&expected_key).unwrap();
    let wrong_id = age::x25519::Identity::generate();
    assert!(
        age::Decryptor::new(&ciphertext[..])
            .unwrap()
            .decrypt(std::iter::once(&wrong_id as &dyn age::Identity))
            .is_err()
    );

    let contents = decrypt_archive_contents(&ciphertext, &fx.identity).unwrap();
    let entries: BTreeSet<String> = contents.keys().cloned().collect();
    for included in [
        "instrukt.yaml",
        "agents.yaml",
        "secrets.env",
        "state/machine-id",
        "state/instrukt.db",
        "state/custom.sqlite",
        "state/eval-fingerprint/p.json",
        "state/agents-deployment.json",
        "state/known-issues/demo.json",
        "state/company-generation/acme.json",
        "git/index.md",
        "explore/report.md",
        "companies/acme/company.yaml",
        "companies/acme/secrets.env",
        "companies/acme/.instrukt/knowledge-proposals.db",
    ] {
        assert!(
            entries.contains(included),
            "expected `{included}` in archive: {entries:?}"
        );
    }

    // Verify actual SQLite rows captured from the live uncheckpointed WAL database.
    let decrypted_db = fx.env.path("decrypted-instrukt.db");
    std::fs::write(&decrypted_db, &contents["state/instrukt.db"]).unwrap();
    {
        let conn = Connection::open(&decrypted_db).unwrap();
        let mut stmt = conn
            .prepare("SELECT body FROM notes ORDER BY rowid")
            .unwrap();
        let rows: Vec<String> = stmt
            .query_map([], |r| r.get(0))
            .unwrap()
            .collect::<Result<Vec<_>, _>>()
            .unwrap();
        assert_eq!(rows, vec!["base-row", "live-wal-row"]);
    }
    let decrypted_company_db = fx.env.path("decrypted-company.db");
    std::fs::write(
        &decrypted_company_db,
        &contents["companies/acme/.instrukt/knowledge-proposals.db"],
    )
    .unwrap();
    {
        let conn = Connection::open(&decrypted_company_db).unwrap();
        let id: i64 = conn
            .query_row("SELECT id FROM p", [], |r| r.get(0))
            .unwrap();
        assert_eq!(id, 1);
    }
    for excluded in [
        "cache/git/repo.bin",
        "docs/general/rule.md",
        "rendered/AGENTS.md",
        "state/index.yaml",
        "state/blueprint/p/g.json",
        "state/instrukt.db.corrupt.1",
        "state/known-issues/demo.lock",
        "locks/docs.lock",
        "git/raw-checkout/code.rs",
        "companies/acme/tracked.md",
        "companies/acme/.instrukt/schemas/s.json",
    ] {
        assert!(
            !entries.contains(excluded),
            "excluded `{excluded}` leaked into archive: {entries:?}"
        );
    }

    let state_path = home.join("state/backup-state.json");
    assert_eq!(
        std::fs::metadata(&state_path).unwrap().permissions().mode() & 0o777,
        0o600
    );
    let state_text = std::fs::read_to_string(&state_path).unwrap();
    assert!(!state_text.contains("SUPER_SECRET_BACKUP_VALUE"));
    let state_json: serde_json::Value = serde_json::from_str(&state_text).unwrap();
    assert_eq!(state_json["last_outcome"], "ok");
    assert_eq!(state_json["last_generation"], expected_key);

    // Negative branch 1: page-corrupted SQLite DB that online_backup copies
    // cleanly (page 1 header intact) but PRAGMA quick_check rejects aborts
    // before publishing and preserves prior generations.
    fx.env.set_env("INSTRUKT_NOW", "2030-01-07T03:00:00Z");
    let broken_db = home.join("state/broken.db");
    {
        let conn = Connection::open(&broken_db).unwrap();
        conn.execute_batch(
            "PRAGMA page_size = 4096;
             PRAGMA journal_mode = DELETE;
             CREATE TABLE entries (id INTEGER PRIMARY KEY, body TEXT);
             CREATE INDEX idx_body ON entries(body);",
        )
        .unwrap();
        for i in 0..200_i64 {
            conn.execute(
                "INSERT INTO entries VALUES (?1, ?2)",
                instrukt_core::state_db::rusqlite::params![
                    i,
                    format!("payload-{i:04}-xxxxxxxxxxxxxxxxxxxxxxxx")
                ],
            )
            .unwrap();
        }
    }
    let mut broken_bytes = std::fs::read(&broken_db).unwrap();
    assert!(broken_bytes.len() > 8192);
    broken_bytes[4096..8192].fill(0xFF);
    std::fs::write(&broken_db, &broken_bytes).unwrap();
    // Prove via SQLite directly that page 1 is intact and online backup copies
    // the database cleanly, while PRAGMA quick_check detects the page-2 corruption.
    let probe_snap = fx.env.path("probe-broken-snap.db");
    {
        let src = Connection::open(&broken_db).unwrap();
        let mut dst = Connection::open(&probe_snap).unwrap();
        instrukt_core::state_db::rusqlite::backup::Backup::new(&src, &mut dst)
            .unwrap()
            .step(-1)
            .unwrap();
        let check: String = dst
            .query_row("PRAGMA quick_check", [], |r| r.get(0))
            .unwrap();
        assert_ne!(check, "ok", "fixture must fail PRAGMA quick_check");
    }

    fx.env.run(fx.env.home(), ["backup", "run"]).failure();
    assert!(fx.s3.get_bytes(&expected_key).is_some());
    assert_eq!(fx.s3.keys().len(), 2);
    let state_after_db_fail: serde_json::Value =
        serde_json::from_slice(&std::fs::read(&state_path).unwrap()).unwrap();
    assert_eq!(state_after_db_fail["last_outcome"], "failed");
    assert_eq!(state_after_db_fail["last_generation"], expected_key);
    assert_eq!(
        state_after_db_fail["last_success_at"],
        "2030-01-06T03:00:00Z"
    );
    std::fs::remove_file(&broken_db).unwrap();

    // Negative branch 2: remote HEAD size mismatch cleans _staging/ and refuses promotion.
    fx.env.set_env("INSTRUKT_NOW", "2030-01-08T03:00:00Z");
    fx.s3.state().head_size_override = Some(999_999);
    fx.env.run(fx.env.home(), ["backup", "run"]).failure();
    fx.s3.state().head_size_override = None;
    assert!(
        !fx.s3.keys().iter().any(|k| k.starts_with("_staging/")),
        "staging must be deleted on HEAD mismatch"
    );
    assert_eq!(fx.s3.keys().len(), 2);

    let notified = std::fs::read_to_string(&fx.notify_log).unwrap();
    let lines: Vec<&str> = notified.lines().collect();
    assert_eq!(
        lines,
        vec![
            format!("backup:failed:{mid}"),
            format!("backup:failed:{mid}")
        ]
    );
}

// spec: project/instrukt/spec/feature/runner/home-backup UC-HB2
#[test]
fn home_backup_list_shows_only_this_machines_generations_newest_first() {
    let fx = BackupFixture::new(5, Some("hosts")).unwrap();
    let home = fx.env.instrukt_home();
    let own_mid = "11112222333344445555666677778888";
    let other_mid = "aaaabbbbccccddddeeeeffff00001111";
    std::fs::write(home.join("state/machine-id"), format!("{own_mid}\n")).unwrap();

    let own_old = format!("hosts/{own_mid}/instrukt-home.20300104T030000Z.tar.zst.age");
    let own_mid_unval = format!("hosts/{own_mid}/instrukt-home.20300105T030000Z.tar.zst.age");
    let own_new = format!("hosts/{own_mid}/instrukt-home.20300106T030000Z.tar.zst.age");
    let other_gen = format!("hosts/{other_mid}/instrukt-home.20300107T030000Z.tar.zst.age");

    fx.s3
        .put_object(&own_old, b"old".to_vec(), "2030-01-04T03:00:00Z");
    fx.s3.put_object(
        &format!("_validated/{own_old}"),
        Vec::new(),
        "2030-01-04T03:00:00Z",
    );
    fx.s3.put_object(
        &own_mid_unval,
        b"unvalidated".to_vec(),
        "2030-01-05T03:00:00Z",
    );
    fx.s3
        .put_object(&own_new, b"newest".to_vec(), "2030-01-06T03:00:00Z");
    fx.s3.put_object(
        &format!("_validated/{own_new}"),
        Vec::new(),
        "2030-01-06T03:00:00Z",
    );
    fx.s3
        .put_object(&other_gen, b"other".to_vec(), "2030-01-07T03:00:00Z");
    fx.s3.put_object(
        &format!("_validated/{other_gen}"),
        Vec::new(),
        "2030-01-07T03:00:00Z",
    );

    let out = fx.env.run(fx.env.home(), ["backup", "list"]).success();
    let stdout = String::from_utf8_lossy(&out.get_output().stdout);
    let lines: Vec<&str> = stdout.lines().collect();
    assert_eq!(lines.len(), 3, "{stdout}");
    assert!(lines[0].contains(&own_new) && lines[0].contains("| validated |"));
    assert!(lines[1].contains(&own_mid_unval) && lines[1].contains("| unvalidated |"));
    assert!(lines[2].contains(&own_old) && lines[2].contains("| validated |"));
    assert!(
        !stdout.contains(other_mid),
        "other machine leaked: {stdout}"
    );
}

// spec: project/instrukt/spec/feature/runner/home-backup UC-HB3
#[test]
fn home_backup_restore_enforces_integrity_confirmation_and_live_home_guards() {
    let mut fx = BackupFixture::new(5, Some("hosts")).unwrap();
    fx.env.set_env("INSTRUKT_NOW", "2030-01-06T03:00:00Z");
    let home = fx.env.instrukt_home();

    {
        let conn = Connection::open(home.join("state/instrukt.db")).unwrap();
        conn.execute_batch(
            "PRAGMA journal_mode=WAL; CREATE TABLE items(name TEXT); INSERT INTO items VALUES ('golden');",
        )
        .unwrap();
    }
    fx.env.run(fx.env.home(), ["backup", "run"]).success();

    std::fs::write(
        home.join("secrets.env"),
        "BACKUP_ACCESS_KEY_ID=AKIA_BACKUP_TEST_KEY\nBACKUP_SECRET_ACCESS_KEY=SUPER_SECRET_BACKUP_VALUE\nEXTRA=mutated\n",
    )
    .unwrap();
    let wal_path = home.join("state/instrukt.db-wal");
    std::fs::write(&wal_path, b"stale-wal").unwrap();

    // 1. Non-interactive without `--yes` fails closed before modifying target.
    let alt_declined = fx.env.path("alt-declined");
    fx.env
        .run(
            fx.env.home(),
            [
                "backup",
                "restore",
                "--into",
                alt_declined.to_str().unwrap(),
                "--identity",
                fx.identity_file.to_str().unwrap(),
            ],
        )
        .failure();
    assert!(!alt_declined.exists());

    // 2. Raw secret key on CLI is refused without echoing it.
    let raw_secret = fx.identity.to_string().expose_secret().to_owned();
    let raw_refused = fx
        .env
        .run(
            fx.env.home(),
            [
                "backup",
                "restore",
                "--into",
                alt_declined.to_str().unwrap(),
                "--identity",
                &raw_secret,
                "--yes",
            ],
        )
        .failure();
    let raw_stderr = String::from_utf8_lossy(&raw_refused.get_output().stderr);
    assert!(!raw_stderr.contains(&raw_secret), "secret leaked to stderr");

    // 3. Wrong private identity fails decryption without modifying destination.
    let wrong_file = fx.env.path("wrong-id.txt");
    let wrong = age::x25519::Identity::generate();
    std::fs::write(
        &wrong_file,
        format!("{}\n", wrong.to_string().expose_secret()),
    )
    .unwrap();
    fx.env
        .run(
            fx.env.home(),
            [
                "backup",
                "restore",
                "--into",
                alt_declined.to_str().unwrap(),
                "--identity",
                wrong_file.to_str().unwrap(),
                "--yes",
            ],
        )
        .failure();
    assert!(!alt_declined.exists());

    // 4. Malicious archive (symlink entry) fails without modifying destination.
    let mut bad_tar = Vec::new();
    {
        let mut builder = tar::Builder::new(&mut bad_tar);
        let mut hdr = tar::Header::new_gnu();
        hdr.set_entry_type(tar::EntryType::Symlink);
        hdr.set_size(0);
        hdr.set_cksum();
        builder
            .append_link(&mut hdr, "escape", "/etc/passwd")
            .unwrap();
        builder.finish().unwrap();
    }
    let mut bad_cipher = Vec::new();
    {
        let rec = fx.identity.to_public();
        let enc =
            age::Encryptor::with_recipients(std::iter::once(&rec as &dyn age::Recipient)).unwrap();
        let w = enc.wrap_output(&mut bad_cipher).unwrap();
        let mut zw = zstd::stream::write::Encoder::new(w, 3).unwrap();
        zw.write_all(&bad_tar).unwrap();
        zw.finish().unwrap().finish().unwrap();
    }
    let bad_key = format!(
        "hosts/{}/instrukt-home.20300105T030000Z.tar.zst.age",
        fx.machine_id().unwrap()
    );
    fx.s3
        .put_object(&bad_key, bad_cipher, "2030-01-05T03:00:00Z");
    fx.env
        .run(
            fx.env.home(),
            [
                "backup",
                "restore",
                "--from",
                &bad_key,
                "--into",
                alt_declined.to_str().unwrap(),
                "--identity",
                fx.identity_file.to_str().unwrap(),
                "--yes",
            ],
        )
        .failure();
    assert!(!alt_declined.exists());

    // 5. Alternate target succeeds even while live-home writer.lock is held,
    //    and live-home restore refuses naming both doors.
    let alt_dir = fx.env.path("restored-alt");
    {
        let _held = LockGuard::try_acquire(&home.join("state/writer.lock"))
            .unwrap()
            .unwrap();
        fx.env
            .run(
                fx.env.home(),
                [
                    "backup",
                    "restore",
                    "--into",
                    alt_dir.to_str().unwrap(),
                    "--identity",
                    fx.identity_file.to_str().unwrap(),
                    "--yes",
                ],
            )
            .success();
        assert!(alt_dir.join("secrets.env").exists());
        assert!(
            std::fs::read_to_string(home.join("secrets.env"))
                .unwrap()
                .contains("EXTRA=mutated")
        );

        let busy = fx
            .env
            .run(
                fx.env.home(),
                [
                    "backup",
                    "restore",
                    "--identity",
                    fx.identity_file.to_str().unwrap(),
                    "--yes",
                ],
            )
            .failure();
        let busy_stderr = String::from_utf8_lossy(&busy.get_output().stderr);
        assert!(busy_stderr.contains("--into"), "{busy_stderr}");
        assert!(
            busy_stderr.contains("stop instrukt services"),
            "{busy_stderr}"
        );
    }

    // 6. Stopped live-home restore commits cleanly, verifies DB, removes stale WAL sidecar.
    fx.env
        .run(
            fx.env.home(),
            [
                "backup",
                "restore",
                "--identity",
                fx.identity_file.to_str().unwrap(),
                "--yes",
            ],
        )
        .success();
    assert!(
        !std::fs::read_to_string(home.join("secrets.env"))
            .unwrap()
            .contains("EXTRA=mutated")
    );
    assert!(!wal_path.exists());
    let conn = Connection::open(home.join("state/instrukt.db")).unwrap();
    let row: String = conn
        .query_row("SELECT name FROM items", [], |r| r.get(0))
        .unwrap();
    assert_eq!(row, "golden");
}

// spec: project/instrukt/spec/feature/runner/home-backup UC-HB5
#[test]
fn home_backup_prunes_unvalidated_before_validated_and_leaves_other_machines_untouched() {
    let mut fx = BackupFixture::new(2, Some("hosts")).unwrap();
    fx.env.set_env("INSTRUKT_NOW", "2030-01-06T03:00:00Z");
    let home = fx.env.instrukt_home();
    let own_mid = "11112222333344445555666677778888";
    let other_mid = "aaaabbbbccccddddeeeeffff00001111";
    std::fs::write(home.join("state/machine-id"), format!("{own_mid}\n")).unwrap();

    let g1_val = format!("hosts/{own_mid}/instrukt-home.20300101T030000Z.tar.zst.age");
    let g2_val = format!("hosts/{own_mid}/instrukt-home.20300102T030000Z.tar.zst.age");
    let g3_unval = format!("hosts/{own_mid}/instrukt-home.20300103T030000Z.tar.zst.age");
    let own_orphan_stg =
        format!("_staging/hosts/{own_mid}/instrukt-home.20300101T030000Z.tar.zst.age");
    let other_gen = format!("hosts/{other_mid}/instrukt-home.20300101T030000Z.tar.zst.age");
    let other_orphan_stg =
        format!("_staging/hosts/{other_mid}/instrukt-home.20300101T030000Z.tar.zst.age");

    for (k, ts) in [
        (&g1_val, "2030-01-01T03:00:00Z"),
        (&g2_val, "2030-01-02T03:00:00Z"),
        (&g3_unval, "2030-01-03T03:00:00Z"),
        (&own_orphan_stg, "2030-01-01T03:00:00Z"),
        (&other_gen, "2030-01-01T03:00:00Z"),
        (&other_orphan_stg, "2030-01-01T03:00:00Z"),
    ] {
        fx.s3.put_object(k, b"fixture".to_vec(), ts);
    }
    for k in [&g1_val, &g2_val, &other_gen] {
        fx.s3.put_object(
            &format!("_validated/{k}"),
            Vec::new(),
            "2030-01-02T03:00:00Z",
        );
    }
    fx.s3
        .put_incomplete_upload("u-own-stale", &own_orphan_stg, "2030-01-01T03:00:00Z");
    fx.s3
        .put_incomplete_upload("u-other-stale", &other_orphan_stg, "2030-01-01T03:00:00Z");

    fx.env.run(fx.env.home(), ["backup", "run"]).success();
    let g4_new = format!("hosts/{own_mid}/instrukt-home.20300106T030000Z.tar.zst.age");

    let keys = fx.s3.keys();
    assert!(
        !keys.contains(&g3_unval),
        "unvalidated must be evicted first: {keys:?}"
    );
    assert!(
        !keys.contains(&g1_val),
        "oldest validated must be evicted second: {keys:?}"
    );
    assert!(
        !keys.contains(&format!("_validated/{g1_val}")),
        "paired _validated marker must be deleted with pruned generation: {keys:?}"
    );
    assert!(
        keys.contains(&g2_val),
        "newer validated must remain: {keys:?}"
    );
    assert!(
        keys.contains(&g4_new),
        "newly published generation must remain: {keys:?}"
    );
    assert!(
        !keys.contains(&own_orphan_stg),
        "own >24h orphan must be deleted"
    );

    assert!(keys.contains(&other_gen));
    assert!(keys.contains(&format!("_validated/{other_gen}")));
    assert!(keys.contains(&other_orphan_stg));
    let st = fx.s3.state();
    assert!(!st.uploads.contains_key("u-own-stale"));
    assert!(st.uploads.contains_key("u-other-stale"));
}
