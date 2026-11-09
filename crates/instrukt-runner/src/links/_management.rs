//! Native caller-bound membership commands and their retained contributions.
use super::{
    DeliveryMode, LinkError, Opening, OpeningPlan, Origin, Pending, active, close_link, deliver,
    directions, failure, link_lock, open_plan, retain, status,
};
use crate::{assignments::WorkLookup, envelopes, runtime::RuntimeAdapter};
use instrukt_core::{config::Env, home::Home, ids::SessionRef, state_db::StateRead};

pub enum Operation<'a> {
    Open {
        target: &'a SessionRef,
        message: Option<&'a str>,
    },
    Close {
        message: Option<&'a str>,
    },
}
pub struct Discussion<'a> {
    pub adapter: &'a dyn RuntimeAdapter,
    pub env: &'a Env,
    pub operation: Operation<'a>,
    pub now: jiff::Timestamp,
}
/// Resolve the native caller and perform only the requested membership operation.
pub fn manage(
    home: &Home,
    work: WorkLookup,
    request: &Discussion<'_>,
) -> Result<String, LinkError> {
    let caller = request
        .adapter
        .caller_identity(request.env)
        .ok_or(crate::sessions::SessionsError::NoActiveRuntime)?
        .session;
    match &request.operation {
        Operation::Open { target, message } => {
            let source = request.adapter.conversation_source(home, request.env)?;
            let caller_ref = &caller;
            let source_identity = source.identity();
            let prepare = message.map(|text| {
                move || {
                    contribution(
                        request,
                        caller_ref,
                        source_identity,
                        text,
                        Origin::Opening {
                            message: text.into(),
                        },
                    )
                }
            });
            let contribution = prepare
                .as_ref()
                .map(|prepare| prepare as &dyn Fn() -> Result<Pending, LinkError>);
            let id = open_plan(
                home,
                source.as_ref(),
                work,
                &OpeningPlan {
                    pair: &Opening {
                        a: &caller,
                        b: target,
                    },
                    contribution,
                },
            )?;
            let result = (|| {
                let _lock = link_lock(home, &id)?;
                if !active(home, &id)? {
                    return Ok(format!("discussion {id} is closed for both members"));
                }
                verify_source(home, request, &id)?;
                let mut direction = directions(home)?
                    .into_iter()
                    .find(|d| d.link == id && d.source == caller)
                    .ok_or_else(|| {
                        LinkError::Refused(
                            "link direction is missing; recover its recorded membership".into(),
                        )
                    })?;
                deliver(
                    home,
                    request.adapter,
                    &mut direction,
                    DeliveryMode::Management,
                )?;
                Ok(format!("discussion {id} is open with {target}"))
            })();
            record_failure(
                home,
                &id,
                &Opening {
                    a: &caller,
                    b: target,
                },
                &result,
            )?;
            result
        }
        Operation::Close { message } => {
            let Some(link) = status(home)?
                .into_iter()
                .find(|link| link.active && (link.a == caller || link.b == caller))
            else {
                return Ok("no active discussion for this thread".into());
            };
            let result = (|| {
                let _lock = link_lock(home, &link.id)?;
                if !active(home, &link.id)? {
                    return Ok(format!("discussion {} is closed for both members", link.id));
                }
                let source = verify_source(home, request, &link.id)?;
                let Some(text) = *message else {
                    close_link(home, &link.id, &caller)?;
                    return Ok(format!("discussion {} is closed for both members", link.id));
                };
                let mut direction = directions(home)?
                    .into_iter()
                    .find(|d| d.link == link.id && d.source == caller)
                    .ok_or_else(|| {
                        LinkError::Refused(
                            "link direction is missing; recover its recorded membership".into(),
                        )
                    })?;
                let key = format!("closing:{}", link.id);
                if let Some(pending) = direction
                    .pending
                    .iter()
                    .find(|p| matches!(p.origin, Origin::Closing { .. }))
                {
                    if !matches!(&pending.origin,Origin::Closing{message} if message==text) {
                        return Err(LinkError::Refused("a different final contribution is already retained; retry its original text or close without a message".into()));
                    }
                } else {
                    let mut final_message = contribution(
                        request,
                        &caller,
                        &source,
                        text,
                        Origin::Closing {
                            message: text.into(),
                        },
                    )?;
                    final_message.turn = key;
                    direction.pending.insert(0, final_message);
                    retain(home, &direction)?;
                }
                deliver(
                    home,
                    request.adapter,
                    &mut direction,
                    DeliveryMode::Management,
                )?;
                Ok(format!("discussion {} is closed for both members", link.id))
            })();
            let peer = if link.a == caller { &link.b } else { &link.a };
            record_failure(
                home,
                &link.id,
                &Opening {
                    a: &caller,
                    b: peer,
                },
                &result,
            )?;
            result
        }
    }
}
fn contribution(
    request: &Discussion<'_>,
    caller: &SessionRef,
    source: &str,
    text: &str,
    origin: Origin,
) -> Result<Pending, LinkError> {
    let thread = request
        .adapter
        .read_state(caller)
        .map_err(|error| LinkError::Native {
            stage: "author observation",
            diagnostic: Box::new(request.adapter.diagnostic("show", &error)),
        })?;
    let author = envelopes::agent(thread.provider.as_deref(), thread.model.as_deref());
    Ok(Pending {
        turn: String::new(),
        content: envelopes::peer(
            &format!("{source}:{}", caller.id),
            &author,
            &envelopes::minute(Some(request.now)),
            text,
        ),
        sending: false,
        origin,
    })
}
fn verify_source(home: &Home, request: &Discussion<'_>, id: &str) -> Result<String, LinkError> {
    let db = StateRead::open(home)?
        .ok_or_else(|| LinkError::Refused("link state disappeared".into()))?;
    let recorded: String = db.conn().query_row(
        "SELECT source_identity FROM conversation_links WHERE id=?1",
        [id],
        |r| r.get(0),
    )?;
    let selected = request.adapter.conversation_source(home, request.env)?;
    if selected.identity() != recorded {
        return Err(LinkError::Refused(
            "selected conversation source changed; restore the recorded source".into(),
        ));
    }
    Ok(recorded)
}
fn record_failure(
    home: &Home,
    id: &str,
    pair: &Opening<'_>,
    result: &Result<String, LinkError>,
) -> Result<(), LinkError> {
    if let Err(error) = result {
        super::_diagnostics::management_failure(home, id, pair, error)?;
        failure(home, id, Some(&pair.a.to_string()), &error.to_string())?;
    }
    Ok(())
}
