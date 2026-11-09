use super::Report;
use instrukt_core::{config::Feature, publication::PublishedRecord};
use instrukt_core::{
    config::{Configuration, Module},
    home::Home,
};
pub(super) struct SourceDemand {
    pub project: instrukt_core::ids::ProjectSlug,
    pub config: Configuration,
}
pub(super) struct WorkDemand {
    pub project: instrukt_core::ids::ProjectSlug,
    pub repository: String,
    pub board: Option<instrukt_core::config::BoardRef>,
}
pub(super) struct Demand {
    pub work: Vec<WorkDemand>,
    pub todos: Vec<String>,
    pub events: Vec<String>,
    pub history: Vec<String>,
    pub history_owned_redis: Vec<String>,
    pub sources: Vec<SourceDemand>,
    pub complete: bool,
    pub redis_references: Vec<String>,
}
impl Demand {
    pub fn projects(&self) -> Vec<String> {
        let mut names = self.todos.clone();
        names.extend(self.events.clone());
        names.sort();
        names.dedup();
        names
    }
}
pub(super) fn read(
    home: &Home,
    cwd: &std::path::Path,
    report: &mut Report,
) -> anyhow::Result<Demand> {
    let mut demand = Demand {
        work: vec![],
        todos: vec![],
        events: vec![],
        history: vec![],
        history_owned_redis: vec![],
        sources: vec![],
        complete: true,
        redis_references: Vec::new(),
    };
    let current = instrukt_core::git::worktree_root(cwd)?;
    let current_project = current
        .as_ref()
        .map(|root| Configuration::load(home, Some(root)))
        .transpose()?
        .and_then(|c| c.project().transpose())
        .transpose()?;
    for record in PublishedRecord::load_all(home)? {
        let result = (|| -> anyhow::Result<()> {
            let config = if home.is_inspection() {
                if current_project.as_ref() == Some(&record.project) {
                    Configuration::load(home, current.as_deref())?
                } else {
                    Configuration::load_recorded_project(home, &record.project, &record.checkout)?
                }
            } else {
                anyhow::ensure!(
                    record.checkout.join("instrukt.yaml").is_file(),
                    "declaring checkout/configuration unavailable; restore project {} before shared withdrawal",
                    record.project
                );
                Configuration::load(home, Some(&record.checkout))?
            };
            anyhow::ensure!(
                config.project()?.as_ref() == Some(&record.project),
                "registered project identity differs from checkout"
            );
            if config
                .project_local_redis()?
                .as_deref()
                .map(|url| owned_server(url, &home.state_dir().join("redis/redis.sock")))
                .transpose()?
                .unwrap_or(false)
            {
                demand.redis_references.push(record.project.to_string());
            }
            let todos = config.module_enabled(Module::Todos)?;
            let events = config.module_enabled(Module::Events)?;
            if config.module_enabled(Module::History)? {
                demand.history.push(record.project.to_string());
                if config
                    .cluster_redis_url()?
                    .as_deref()
                    .map(|url| owned_server(url, &home.state_dir().join("redis/redis.sock")))
                    .transpose()?
                    .unwrap_or(false)
                {
                    demand.history_owned_redis.push(record.project.to_string());
                    demand.redis_references.push(record.project.to_string());
                }
            }
            let repository = if todos {
                demand.todos.push(record.project.to_string());
                if home.is_inspection() && current_project.as_ref() != Some(&record.project) {
                    anyhow::ensure!(
                        config.github_repository()?.is_some(),
                        "missing recorded GitHub repository; run instrukt install"
                    );
                }
                Some(instrukt_core::github::repository(
                    &config,
                    config.github_api_url()?,
                    &record.checkout,
                )?)
            } else {
                None
            };
            if let Some(repository) = &repository {
                demand.work.push(WorkDemand {
                    project: record.project.clone(),
                    repository: repository.clone(),
                    board: config.board()?,
                });
            }
            if events {
                demand.events.push(record.project.to_string());
            }
            let sources = config.webhooks()?;
            anyhow::ensure!(
                events || sources.is_empty(),
                "webhook sources with events off; explicitly enable events or remove sources"
            );

            let features = if todos {
                vec![Feature::TodoDispatch, Feature::RecurringTasks]
            } else if events {
                vec![Feature::StandingAgents]
            } else {
                vec![]
            };
            if let Some(unavailable) = config.unavailable(&features)? {
                anyhow::bail!("{}", unavailable.describe(&features));
            }
            if !home.is_inspection() {
                config.record_project(&record.project, repository.as_deref())?;
            }
            if !sources.is_empty() {
                demand.sources.push(SourceDemand {
                    project: record.project.clone(),
                    config,
                });
            }
            Ok(())
        })();
        if let Err(error) = result {
            demand.complete = false;
            report.finding(format!("project {}", record.project), error);
        }
    }
    Ok(demand)
}

fn owned_server(url: &str, owned: &std::path::Path) -> anyhow::Result<bool> {
    use redis::IntoConnectionInfo;
    let info = url.into_connection_info().map_err(|_| {
        anyhow::anyhow!("invalid local Redis reference; repair it before withdrawal")
    })?;
    let redis::ConnectionAddr::Unix(socket) = info.addr() else {
        return Ok(false);
    };
    let canonical = |path: &std::path::Path| {
        path.parent()
            .and_then(|p| p.canonicalize().ok())
            .zip(path.file_name())
            .map(|(parent, name)| parent.join(name))
            .unwrap_or_else(|| path.into())
    };
    Ok(canonical(socket) == canonical(owned))
}
