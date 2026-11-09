use super::*;
use instrukt_core::{config::Feature, publication::PublishedRecord};
pub(super) struct Demand {
    pub todos: Vec<String>,
    pub events: Vec<String>,
    pub sources: Vec<PublishedRecord>,
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
pub(super) fn read(home: &Home, report: &mut Report) -> anyhow::Result<Demand> {
    let mut demand = Demand {
        todos: vec![],
        events: vec![],
        sources: vec![],
        complete: true,
        redis_references: Vec::new(),
    };
    for record in PublishedRecord::load_all(home)? {
        let result = (|| -> anyhow::Result<()> {
            anyhow::ensure!(
                record.checkout.join("instrukt.yaml").is_file(),
                "declaring checkout/configuration unavailable at {}; restore it before shared withdrawal",
                record.checkout.display()
            );
            let layer = instrukt_core::config::Layer::Project(record.checkout.clone());
            if home.is_inspection()
                && let Some(notice) = instrukt_core::config::inspect_layer(home, &layer)?.migration
            {
                report.notices.push(notice);
            }
            let config = Configuration::load(home, Some(&record.checkout))?;
            anyhow::ensure!(
                config.project()?.as_ref() == Some(&record.project),
                "registered project identity differs from checkout"
            );
            if config.project_local_redis()?.as_deref()
                == Some(&format!(
                    "unix://{}",
                    home.state_dir().join("redis/redis.sock").display()
                ))
            {
                demand.redis_references.push(record.project.to_string());
            }
            let todos = config.module_enabled(Module::Todos)?;
            let events = config.module_enabled(Module::Events)?;
            if todos {
                demand.todos.push(record.project.to_string());
                instrukt_core::github::repository(
                    &config,
                    config.github_api_url()?,
                    &record.checkout,
                )?;
            }
            if events {
                demand.events.push(record.project.to_string());
            }
            let sources = config.webhooks()?;
            anyhow::ensure!(
                events || sources.is_empty(),
                "webhook sources with events off; explicitly enable events or remove sources"
            );
            if !sources.is_empty() {
                demand.sources.push(record.clone());
            }
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
            Ok(())
        })();
        if let Err(error) = result {
            demand.complete = false;
            report.finding(&format!("project {}", record.project), error);
        }
    }
    Ok(demand)
}
