//! The machine-owned T3 state location, independent of instrukt's output home.
use instrukt_core::{config::Env, home::Home};
use std::path::PathBuf;

pub(crate) struct T3Home {
    directory: PathBuf,
}
impl T3Home {
    pub(crate) fn discover(home: &Home, env: &Env) -> Self {
        let explicit = env.get("T3CODE_HOME").filter(|s| !s.trim().is_empty());
        let base = explicit
            .map(|s| {
                let s = s.trim();
                if let Some(rest) = s.strip_prefix("~/") {
                    home.machine_user_home().join(rest)
                } else {
                    PathBuf::from(s)
                }
            })
            .unwrap_or_else(|| home.machine_user_home().join(".t3"));
        let band = if explicit.is_none()
            && env
                .get("VITE_DEV_SERVER_URL")
                .is_some_and(|s| !s.is_empty())
        {
            "dev"
        } else {
            "userdata"
        };
        let directory = base.join(band);
        // Existing state has one physical identity; a missing location remains discoverable on creation.
        let directory = directory.canonicalize().unwrap_or(directory);
        Self { directory }
    }
    pub(crate) fn database(&self) -> PathBuf {
        self.directory.join("statev2.sqlite")
    }
    pub(crate) fn descriptor(&self) -> PathBuf {
        self.directory.join("server-runtime.json")
    }
}
