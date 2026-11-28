-- Golden v43 state database seed covering every Lead 2 migration row shape
-- for `state_db v43 -> v44` (used by `UC-SD4` migration tests):
--   1. Live `turn_output` binding (pre-v34 `opening_conversation IS NULL`) + multi-runtime PK disambiguation
--   2. Crash-window `turn_output` binding (`session IS NULL`, matching `runtime_bindings` row exists)
--   3. Unstarted cached `turn_output` row (`session IS NULL`, no `runtime_bindings` row, `request IS NOT NULL`)
--   4. Live `explicit` binding (`needs_explicit_rollover` remains 0)
--   5. Company-scoped `turn_output` binding (`session_scope = 'company'`, `opening_conversation = 'C5/1'`)
--   6. Pre-v29 binding with `start_key IS NULL` (`dispatch_command_id` remains NULL)

INSERT INTO runtime_bindings(runtime,semantic_key,kind,project,subject,session,checkout,accepted,request,generation,start_key)
VALUES
    ('t3','k:live','conversation','acme','k:live','t3:sess-live','/acme',1,'{}',1,'cmd-live-start'),
    ('other','k:live','conversation','acme','k:live','other:sess-old','/acme',1,'{}',1,'cmd-wrong-runtime'),
    ('t3','k:crash','conversation','acme','k:crash','t3:sess-crash','/acme',0,'{}',1,'cmd-crash-start'),
    ('t3','k:explicit','conversation','acme','k:explicit','t3:sess-exp','/acme',1,'{}',1,'cmd-exp-start'),
    ('t3','k:company','conversation','acme','k:company','t3:sess-co','/acme',1,'{}',1,'cmd-co-start'),
    ('t3','k:prev29','conversation','acme','k:prev29','t3:sess-prev29','/acme',1,'{}',1,NULL);

INSERT INTO connected_conversations(company,connector,conversation,semantic_key,opening,opening_conversation,request,session,source_identity,position_json,reply_mode,session_scope)
VALUES
    ('acme','slack','C1/1','k:live','m-live',NULL,'{"brief":"old"}','t3:sess-live','src-1','{}','turn_output','conversation'),
    ('acme','slack','C2/1','k:crash','m-crash','C2/1','{"brief":"crash"}',NULL,'src-2','{}','turn_output','conversation'),
    ('acme','slack','C3/1','k:unstarted','m-unstarted','C3/1','{"brief":"stale"}',NULL,NULL,NULL,'turn_output','conversation'),
    ('acme','coder','C4/1','k:explicit','m-exp','C4/1','{"brief":"exp"}','t3:sess-exp','src-4','{}','explicit','conversation'),
    ('acme','co','','k:company','m-co','C5/1','{"brief":"co"}','t3:sess-co','src-5','{}','turn_output','company'),
    ('acme','slack','C6/1','k:prev29','m-prev29',NULL,'{"brief":"old"}','t3:sess-prev29','src-6','{}','turn_output','conversation');

INSERT INTO conversation_messages(company,connector,conversation,message,written_us,written_at,arrived_at,state,text,turn_update,dispatch_command_id)
VALUES
    ('acme','slack','C1/1','m-live',1,'2026-11-27T10:00:00Z','2026-11-27T10:00:00Z','sent','Hi','working',NULL),
    ('acme','slack','C1/1','m-followup',2,'2026-11-27T10:01:00Z','2026-11-27T10:01:00Z','sent','More',NULL,'cmd-followup'),
    ('acme','coder','C4/1','m-exp',3,'2026-11-27T10:02:00Z','2026-11-27T10:02:00Z','sent','Exp',NULL,NULL),
    ('acme','co','C5/1','m-co',4,'2026-11-27T10:03:00Z','2026-11-27T10:03:00Z','sent','Co',NULL,NULL),
    ('acme','slack','C6/1','m-prev29',5,'2026-11-27T10:04:00Z','2026-11-27T10:04:00Z','sent','Old',NULL,NULL);

INSERT INTO conversation_answers(company,connector,conversation,turn,piece,text,posted_at)
VALUES
    ('acme','coder','C4/1','reply:abc',0,'Done','2026-11-27T10:05:00Z');
