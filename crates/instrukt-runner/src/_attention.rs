//! Deliver recorded approvals to the current live claimant after work transactions.
use crate::inputs::{self, Entry, InputKind, Intent};
use crate::keyed::RuntimeInvocation;
use crate::runtime::RuntimeAdapter;
use crate::sessions::{SessionState, SessionsError};
use crate::state::{self, RunnerState, StateError};
use instrukt_core::ids::SessionRef;
use instrukt_core::state_db::rusqlite::{OptionalExtension, params};
use jiff::Timestamp;

pub(crate) fn approvals(
    invocation: &RuntimeInvocation,
    adapter: &dyn RuntimeAdapter,
    now: Timestamp,
) -> Result<(), SessionsError> {
    let state = RunnerState::new(invocation.home());
    let notices=state.read(|conn,path|{
        let mut q=conn.prepare("SELECT a.todo_id,a.decision_sequence,a.content,CASE WHEN EXISTS (SELECT 1 FROM session_bindings b WHERE b.todo_id=t.id AND b.session=t.claim_session AND b.released_at IS NULL AND json_extract(b.evidence,'$.assignment')='task') THEN NULL ELSE t.claim_session END FROM approval_attention a LEFT JOIN todos t ON t.id=a.todo_id WHERE a.state='pending'").map_err(state::sql(path))?;
        q.query_map([],|r|Ok((r.get::<_,String>(0)?,r.get::<_,u64>(1)?,r.get::<_,String>(2)?,r.get::<_,Option<String>>(3)?))).map_err(state::sql(path))?.collect::<Result<Vec<_>,_>>().map_err(state::sql(path))
    })?.unwrap_or_default();
    for (todo, seq, content, claim) in notices {
        let result = (|| -> Result<(), SessionsError> {
            let Some(claim) = claim else {
                return finish_approval(&state, &todo, seq, "no-live-claim", None)
                    .map_err(Into::into);
            };
            let target =
                SessionRef::parse(&claim).map_err(|e| SessionsError::Refused(e.to_string()))?;
            if adapter.read_state(&target)?.state == SessionState::Gone {
                return finish_approval(&state, &todo, seq, "no-live-claim", None)
                    .map_err(Into::into);
            }
            let id = format!("approval:{todo}:{seq}");
            let notice: serde_json::Value = serde_json::from_str(&content).map_err(|e| {
                SessionsError::Refused(format!(
                    "approval attention {todo}/{seq} is unreadable: {e}"
                ))
            })?;
            let arrived_at = notice
                .get("arrived_at")
                .and_then(|v| v.as_str())
                .map(str::parse::<Timestamp>)
                .transpose()
                .map_err(|e| SessionsError::Refused(e.to_string()))?
                .unwrap_or(now);
            state.write::<_,StateError>(|tx|{
                // Revalidate the claim in the transaction that binds the queued input.
                let current=tx.query_row("SELECT claim_session FROM todos WHERE id=?1",[&todo],|r|r.get::<_,Option<String>>(0)).optional().map_err(state::sql(&state::db_path(tx)))?.flatten();
                if current.as_deref()!=Some(&claim){return Ok(());}
                inputs::retain(tx,&target,&Entry{id:&id,kind:InputKind::Approval, assignment:None,source_at:notice.get("source_at").or_else(||notice.get("at")).and_then(|v|v.as_str()),arrived_at,intent:Intent::Asap,content:&content})?;
                tx.execute("UPDATE approval_attention SET target=?3,input_id=?4,failure=NULL WHERE todo_id=?1 AND decision_sequence=?2",params![todo,seq,claim,id]).map_err(state::sql(&state::db_path(tx)))?;Ok(())
            })?;
            if inputs::outcome(&state, &target, &id)? != inputs::InputOutcome::Pending {
                finish_approval(&state, &todo, seq, "accepted", None)?;
            }
            Ok(())
        })();
        if let Err(error) = result {
            finish_approval(&state, &todo, seq, "pending", Some(&error.to_string()))?;
            tracing::warn!(event="approval_attention_failed",todo=%todo,error=%error);
        }
    }
    Ok(())
}
fn finish_approval(
    state: &RunnerState,
    todo: &str,
    seq: u64,
    outcome: &str,
    failure: Option<&str>,
) -> Result<(), StateError> {
    state.write(|tx|{tx.execute("UPDATE approval_attention SET state=?3,failure=?4 WHERE todo_id=?1 AND decision_sequence=?2",params![todo,seq,outcome,failure]).map_err(state::sql(&state::db_path(tx)))?;Ok(())})
}

/// A synchronous work command attempts attention only after its work transaction closed.
pub fn deliver_approval_attention(
    home: &instrukt_core::home::Home,
    now: Timestamp,
) -> Result<(), SessionsError> {
    let Some(runtime) =
        instrukt_core::config::Configuration::load_global(home)?.runner_runtime()?
    else {
        return Ok(());
    };
    let Some(invocation) = RuntimeInvocation::try_acquire(home)? else {
        return Ok(());
    };
    let adapter = crate::runtime::adapter(runtime);
    approvals(&invocation, adapter.as_ref(), now)?;
    inputs::pump_all(&invocation, adapter.as_ref(), now)?;
    settle_approvals(&RunnerState::new(home))?;
    Ok(())
}

pub(crate) fn settle_approvals(state: &RunnerState) -> Result<(), StateError> {
    state.write(|tx|{
        tx.execute("UPDATE approval_attention SET failure=(SELECT i.failure FROM runtime_inputs i WHERE i.input_id=approval_attention.input_id AND (i.runtime || ':' || i.session)=approval_attention.target), state=CASE WHEN EXISTS (SELECT 1 FROM runtime_inputs i WHERE i.input_id=approval_attention.input_id AND (i.runtime || ':' || i.session)=approval_attention.target AND i.state IN ('accepted','entered')) THEN 'accepted' ELSE state END WHERE state='pending'",[]).map_err(state::sql(&state::db_path(tx)))?;Ok(())
    })
}
