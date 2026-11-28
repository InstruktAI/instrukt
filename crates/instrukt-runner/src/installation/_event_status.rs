//! Operator read-model for `instrukt events status`: retained state-database
//! facts, admission summary, proposal findings, Redis/service inspection, and
//! merged status JSON.

use instrukt_core::config::{Configuration, Module};
use instrukt_core::state_db::rusqlite::params;

use super::{
    _demand, _proposals, _redis, InstallationError, Report, Request, converge, inspection_error,
    runner_service,
};

/// Read retained transport/activation facts without enabling events or creating groups.
fn retained_event_status(request: &Request<'_>) -> Result<serde_json::Value, InstallationError> {
    let state = crate::state::RunnerState::new(request.home);
    // What the runner keeps in its state is reported whatever the files say
    // now: an invalid machine layer must not hide why the runner holds on.
    let failures = state
        .read(|conn, path| {
            let mut stmt = conn
                .prepare("SELECT event_id, state, failure FROM procedure_activations WHERE failure IS NOT NULL AND state!='set_aside'")
                .map_err(crate::state::sql(path))?;
            let rows = stmt
                .query_map([], |row| {
                    Ok(serde_json::json!({
                        "id": row.get::<_, String>(0)?,
                        "state": row.get::<_, String>(1)?,
                        "failure": row.get::<_, String>(2)?,
                    }))
                })
                .map_err(crate::state::sql(path))?;
            rows.collect::<Result<Vec<_>, _>>()
                .map_err(crate::state::sql(path))
        })?
        .unwrap_or_default();
    // What the retained consumers hold whatever the files say now.
    let consumers = crate::consumers::status(request.home, &state)?;
    let stopped_executions = crate::executions::Executions::new(&state).stopped()?;
    let input_findings = state
        .read(|conn, path| {
            let sql = crate::state::sql(path);
            let mut stmt = conn
                .prepare("SELECT runtime,session,input_id,state,failure,evidence_source,evidence_at FROM runtime_inputs WHERE state='unconfirmed' OR (state='accepted' AND waiting=1 AND failure IS NOT NULL) ORDER BY rowid")
                .map_err(&sql)?;
            stmt.query_map([], |row| {
                Ok(serde_json::json!({
                    "session": format!("{}:{}", row.get::<_, String>(0)?, row.get::<_, String>(1)?),
                    "input_id": row.get::<_, String>(2)?,
                    "state": row.get::<_, String>(3)?,
                    "failure": row.get::<_, Option<String>>(4)?,
                    "evidence_source": row.get::<_, Option<String>>(5)?,
                    "evidence_at": row.get::<_, Option<String>>(6)?,
                }))
            })
            .map_err(&sql)?
            .collect::<Result<Vec<_>, _>>()
            .map_err(sql)
        })?
        .unwrap_or_default();
    let set_aside = state
        .read(|conn, path| {
            let sql = crate::state::sql(path);
            // A mirrored activation and its input account for one retained entry.
            let rows = "SELECT event_id AS id, accepted_at AS at, failure FROM procedure_activations WHERE state='set_aside'
                UNION ALL SELECT i.input_id, i.arrived_at, i.failure FROM runtime_inputs i WHERE i.state='set_aside'
                AND NOT EXISTS (SELECT 1 FROM standing_deliveries d WHERE d.input_id=i.input_id AND d.target=(i.runtime || ':' || i.session) AND d.state='set_aside')";
            let count: u64 = conn
                .query_row(&format!("SELECT count(*) FROM ({rows})"), [], |row| {
                    row.get(0)
                })
                .map_err(&sql)?;
            let mut statement = conn
                .prepare(&format!(
                    "SELECT id,at,failure FROM ({rows}) ORDER BY julianday(at) DESC,id LIMIT ?1"
                ))
                .map_err(&sql)?;
            let recent = statement
                .query_map([crate::inputs::SET_ASIDE_STATUS_LIMIT], |row| {
                    Ok(serde_json::json!({
                        "id": row.get::<_, String>(0)?,
                        "at": row.get::<_, String>(1)?,
                        "failure": row.get::<_, Option<String>>(2)?,
                    }))
                })
                .map_err(&sql)?
                .collect::<Result<Vec<_>, _>>()
                .map_err(sql)?;
            Ok(serde_json::json!({"count": count, "recent": recent}))
        })?
        .unwrap_or_else(|| serde_json::json!({"count": 0, "recent": []}));
    let undeliverable_verdict_notices = state
        .read(|conn, path| {
            let sql = crate::state::sql(path);
            let verdict_kind = crate::inputs::InputKind::Verdict.name();
            let count: u64 = conn
                .query_row(
                    "SELECT count(*) FROM runtime_inputs WHERE kind=?1 AND state='failed'",
                    params![verdict_kind],
                    |row| row.get(0),
                )
                .map_err(&sql)?;
            let mut statement = conn
                .prepare(
                    "SELECT runtime, session, input_id, arrived_at, failure FROM runtime_inputs WHERE kind=?1 AND state='failed' ORDER BY julianday(arrived_at) DESC, input_id LIMIT ?2",
                )
                .map_err(&sql)?;
            let recent = statement
                .query_map(
                    params![verdict_kind, crate::inputs::SET_ASIDE_STATUS_LIMIT],
                    |row| {
                        Ok(serde_json::json!({
                            "session": format!("{}:{}", row.get::<_, String>(0)?, row.get::<_, String>(1)?),
                            "input_id": row.get::<_, String>(2)?,
                            "at": row.get::<_, String>(3)?,
                            "failure": row.get::<_, Option<String>>(4)?,
                        }))
                    },
                )
                .map_err(&sql)?
                .collect::<Result<Vec<_>, _>>()
                .map_err(sql)?;
            Ok(serde_json::json!({"count": count, "recent": recent}))
        })?
        .unwrap_or_else(|| serde_json::json!({"count": 0, "recent": []}));
    // The runner workers' current failures, while they last: the per-worker
    // failure files under `Home::runner_workers_dir` and `Home::computer_fleet_file`.
    let mut workers: Vec<serde_json::Value> = crate::state::read_worker_failures(request.home)?
        .into_iter()
        .map(crate::state::WorkerFailure::into_json)
        .collect();
    workers.extend(crate::computer_fleet::failure_status(request.home));
    workers.sort_by(|a, b| a["worker"].as_str().cmp(&b["worker"].as_str()));
    // The run loop's last skipped wake, read lock-free from
    // `Home::runner_skipped_wake_file` (or an un-migrated legacy row).
    let skipped_wake =
        crate::state::read_skipped_wake(request.home)?.map(crate::state::SkippedWake::into_json);
    // What the running runner last did with the configuration it admitted:
    // replaced its generation, or kept the one in force, read lock-free from
    // `Home::runner_admission_file` (or an un-migrated legacy row).
    let runner = crate::state::read_runner_admission(request.home)?
        .map(crate::state::RunnerAdmission::into_json);
    let conversations = request
        .company
        .map(|company| company.status(request.home, request.now))
        .transpose()
        .map_err(|e| inspection_error("conversations", e))?
        .unwrap_or(serde_json::Value::Null);
    Ok(serde_json::json!({
        "runner": runner,
        "workers": workers,
        "skipped_wake": skipped_wake,
        "activation_failures": failures,
        "consumers": consumers,
        "stopped_executions": stopped_executions,
        "input_findings": input_findings,
        "set_aside": set_aside,
        "undeliverable_verdict_notices": undeliverable_verdict_notices,
        "conversations": conversations,
    }))
}

pub fn events_status(request: &Request<'_>) -> Result<serde_json::Value, InstallationError> {
    let home = request.home.for_inspection();
    let request = &Request {
        home: &home,
        ..request.clone()
    };
    let mut status = match retained_event_status(request) {
        Ok(status) => status,
        Err(InstallationError::State(crate::state::StateError::Db(
            error @ (instrukt_core::state_db::StateDbError::Newer { .. }
            | instrukt_core::state_db::StateDbError::Older { .. }),
        ))) => {
            serde_json::json!({"state_database": {"path": home.state_db_file(), "reason": error.to_string()}})
        }
        Err(error) => return Err(error),
    };
    let runner = status["runner"].take();
    if let Some(fields) = status.as_object_mut() {
        fields.remove("runner");
    }
    // This computer's own machine ID, as it reads it, for comparison with the
    // other computers; reading creates no ID.
    let computer = instrukt_core::machine::existing_machine_id(request.home)
        .map_err(|e| inspection_error("machine ID", e))?
        .map(|id| id.as_str().to_owned());
    // What the files admit now: each project's admitted modules, the findings
    // of what is left out, or why no generation can be admitted at all.
    let candidate = instrukt_core::config::Admitted::load(request.home);
    let notification_configured = candidate.as_ref().ok().and_then(|candidate| {
        candidate
            .generation
            .global()
            .runner_notify()
            .ok()
            .map(|notify| matches!(notify, instrukt_core::config::Notify::Command(_)))
    });
    let admission = match &candidate {
        Ok(candidate) => serde_json::json!({
            "error": null,
            "runner": runner,
            "projects": candidate.generation.projects().iter().map(|project| serde_json::json!({
                "project": project.slug.as_str(),
                "todos": project.todos,
                "events": project.events,
                "history": project.history,
            })).collect::<Vec<_>>(),
            "findings": candidate.findings.iter().map(|finding| serde_json::json!({
                "document": finding.document,
                "key": finding.key,
                "message": finding.message,
            })).collect::<Vec<_>>(),
        }),
        Err(error) => {
            serde_json::json!({"error": error.to_string(), "runner": runner, "projects": [], "findings": []})
        }
    };
    let backup = crate::backup::events_status_json(request.home, request.now);
    if candidate.is_err() {
        // Nothing read from the full configuration can be shown: what depends
        // on it is absent, while local backup standing and the admission error
        // still surface.
        return Ok(merge_status(
            status,
            serde_json::json!({
                "computer": computer,
                "ingress": null,
                "enabled_projects": null,
                "transport": null,
                "infrastructure": null,
                "findings": [],
                "admission": admission,
                "notification_configured": notification_configured,
                "runner_service": null,
                "backup": backup,
            }),
        ));
    }
    let mut report = Report::default();
    if let Ok(candidate) = &candidate {
        _proposals::findings(request, candidate, &mut report);
    }
    let demand = _demand::read(request.home, request.cwd, &mut report)
        .map_err(|e| InstallationError::Demand(e.to_string()))?;
    let infrastructure = converge(&Request {
        check: true,
        module: Some(Module::Events),
        ..request.clone()
    })?;
    // Only the runner service's state is shown: it is read alone, without
    // asking the runtime about anything the status leaves out.
    let mut service = Report::default();
    runner_service(
        &Request {
            check: true,
            module: None,
            conversations: true,
            ..request.clone()
        },
        &demand,
        &mut service,
    );
    let runner_service = service.runner_service;
    let global = Configuration::load_global(request.home)?.get(None, request.env)?;
    let configured = global.get("redis").is_some();
    let redis = if demand.events.is_empty() && !configured {
        vec![]
    } else {
        _redis::inspect(request, &mut report).map_err(|e| inspection_error("Redis", e))?
    };
    // This computer's own two ingress settings, as it reads them.
    let setting = |key: &str| {
        global
            .get("ingress")
            .and_then(|ingress| ingress.get(key))
            .and_then(serde_yaml_ng::Value::as_str)
            .map(str::to_owned)
    };
    Ok(merge_status(
        status,
        serde_json::json!({
            "computer": computer,
            "ingress": {"computer": setting("computer"), "reactions": setting("reactions")},
            "enabled_projects": demand.events,
            "transport": redis,
            "infrastructure": infrastructure,
            "findings": report.findings,
            "admission": admission,
            "notification_configured": notification_configured,
            "runner_service": runner_service,
            "backup": backup,
        }),
    ))
}

fn merge_status(
    mut retained: serde_json::Value,
    mut configured: serde_json::Value,
) -> serde_json::Value {
    if let (Some(retained), Some(configured)) =
        (retained.as_object_mut(), configured.as_object_mut())
    {
        retained.append(configured);
    }
    retained
}
