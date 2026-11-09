//! An owned effective receiver invocation, separate from native socket custody.
use instrukt_core::home::Home;
use serde::{Deserialize, Serialize};
use std::{collections::BTreeMap, path::PathBuf};
#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct ReceiverLaunch {
    schema_version: u32,
    pub executable: PathBuf,
    pub environment: BTreeMap<String, String>,
    port: u16,
}
impl ReceiverLaunch {
    pub fn file(home: &Home, port: u16) -> PathBuf {
        home.state_dir()
            .join("receiver")
            .join(format!("{port}.json"))
    }
    pub fn desired(request: &super::Request<'_>, port: u16) -> Self {
        Self {
            schema_version: 1,
            executable: request.wake.executable.into(),
            port,
            environment: BTreeMap::from([
                (
                    "HOME".into(),
                    request.home.machine_user_home().display().to_string(),
                ),
                (
                    "INSTRUKT_HOME".into(),
                    request.home.root().display().to_string(),
                ),
                (
                    "INSTRUKT_DEVELOPMENT".into(),
                    u8::from(request.home.is_development()).to_string(),
                ),
                ("PATH".into(), request.wake.path_env.into()),
            ]),
        }
    }
    pub fn encode(&self) -> anyhow::Result<String> {
        Ok(serde_json::to_string_pretty(self)?)
    }
    pub fn parse(bytes: &[u8], port: u16) -> anyhow::Result<Self> {
        let value: Self = serde_json::from_slice(bytes)?;
        anyhow::ensure!(
            value.schema_version == 1 && value.port == port && value.executable.is_absolute(),
            "invalid owned receiver invocation; rerun instrukt install"
        );
        anyhow::ensure!(
            value.environment.keys().all(|key| matches!(
                key.as_str(),
                "HOME" | "INSTRUKT_HOME" | "INSTRUKT_DEVELOPMENT" | "PATH"
            )),
            "invalid receiver environment shape"
        );
        Ok(value)
    }
    pub fn applied(&self, digest: &str) -> bool {
        std::env::var("INSTRUKT_RECEIVER_APPLIED").ok().as_deref() == Some(digest)
            && std::env::current_exe().ok().as_deref() == Some(self.executable.as_path())
            && self
                .environment
                .iter()
                .all(|(key, value)| std::env::var(key).ok().as_ref() == Some(value))
    }
}
pub(crate) fn owned(home: &Home, port: u16) -> anyhow::Result<Option<(ReceiverLaunch, String)>> {
    let file = ReceiverLaunch::file(home, port);
    let Some(contents) = super::_services::owned_receiver_file(home, &file)? else {
        return Ok(None);
    };
    use sha2::{Digest, Sha256};
    let digest = hex::encode(Sha256::digest(contents.as_bytes()));
    Ok(Some((
        ReceiverLaunch::parse(contents.as_bytes(), port)?,
        digest,
    )))
}
