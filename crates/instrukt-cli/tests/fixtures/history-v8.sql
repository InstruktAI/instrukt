-- Frozen V8 input following history-v7.sql.

DROP TABLE history_pending;
CREATE TABLE history_projection_records (
 source TEXT NOT NULL, projection TEXT NOT NULL, native_id TEXT NOT NULL,
 thread_id TEXT NOT NULL, updated_at TEXT NOT NULL, payload_json TEXT NOT NULL,
 PRIMARY KEY(source,projection,native_id)
);
CREATE INDEX history_projection_threads ON history_projection_records(source,thread_id);
CREATE TABLE history_source_positions (source TEXT PRIMARY KEY,position_json TEXT NOT NULL);
INSERT INTO history_projection_records
SELECT source,'conversation',native_id,native_id,coalesce(json_extract(document,'$.thread.updatedAt'),''),
 json_object('kind','conversation','id',native_id,'conversation_id',native_id,'parent_id',json_extract(document,'$.thread.lineage.parentThreadId'),
 'title',json_extract(document,'$.thread.title'),'text',NULL,'run_id',NULL,'role',NULL,
 'created_at',json_extract(document,'$.thread.createdAt'),'updated_at',json_extract(document,'$.thread.updatedAt'),
 'completed',json('false'),'native',json_extract(document,'$.thread'))
FROM history_conversations WHERE source LIKE 't3:%' AND json_type(document,'$.thread')='object';
INSERT OR IGNORE INTO history_projection_records
SELECT c.source,'message',json_extract(v.value,'$.id'),coalesce(json_extract(v.value,'$.threadId'),c.native_id),
 coalesce(json_extract(v.value,'$.updatedAt'),json_extract(v.value,'$.createdAt'),json_extract(v.value,'$.completedAt'),json_extract(v.value,'$.requestedAt'),''),
 json_object('kind','message','id',json_extract(v.value,'$.id'),'conversation_id',coalesce(json_extract(v.value,'$.threadId'),c.native_id),
 'parent_id',NULL,'title',NULL,'run_id',json_extract(v.value,'$.runId'),'role',
 CASE WHEN json_extract(v.value,'$.role') IN ('user','assistant','system') THEN json_extract(v.value,'$.role') ELSE NULL END,
 'created_at',coalesce(json_extract(v.value,'$.createdAt'),json_extract(v.value,'$.requestedAt')),
 'updated_at',coalesce(json_extract(v.value,'$.updatedAt'),json_extract(v.value,'$.createdAt')),
 'completed',CASE WHEN json_extract(v.value,'$.streaming')=1 THEN json('false') ELSE json('true') END,
 'text',coalesce(json_extract(v.value,'$.text'),json_extract(v.value,'$.markdown'),json_extract(v.value,'$.input'),'') , 'native',json(v.value))
FROM history_conversations c,json_each(c.document,'$.messages') v
WHERE c.source LIKE 't3:%' AND json_type(v.value,'$.id')='text';
INSERT OR IGNORE INTO history_projection_records
SELECT c.source,'run',json_extract(v.value,'$.id'),coalesce(json_extract(v.value,'$.threadId'),c.native_id),
 coalesce(json_extract(v.value,'$.updatedAt'),json_extract(v.value,'$.createdAt'),json_extract(v.value,'$.completedAt'),json_extract(v.value,'$.requestedAt'),''),
 json_object('kind','run','id',json_extract(v.value,'$.id'),'conversation_id',coalesce(json_extract(v.value,'$.threadId'),c.native_id),
 'parent_id',NULL,'title',NULL,'run_id',json_extract(v.value,'$.runId'),'role',
 CASE WHEN json_extract(v.value,'$.role') IN ('user','assistant','system') THEN json_extract(v.value,'$.role') ELSE NULL END,
 'created_at',coalesce(json_extract(v.value,'$.createdAt'),json_extract(v.value,'$.requestedAt')),
 'updated_at',coalesce(json_extract(v.value,'$.updatedAt'),json_extract(v.value,'$.createdAt')),
 'completed',CASE WHEN json_extract(v.value,'$.completedAt') IS NOT NULL THEN json('true') ELSE json('false') END,
 'text',coalesce(json_extract(v.value,'$.text'),json_extract(v.value,'$.markdown'),json_extract(v.value,'$.input'),'') , 'native',json(v.value))
FROM history_conversations c,json_each(c.document,'$.runs') v
WHERE c.source LIKE 't3:%' AND json_type(v.value,'$.id')='text';
INSERT OR IGNORE INTO history_projection_records
SELECT c.source,'subagent',json_extract(v.value,'$.id'),coalesce(json_extract(v.value,'$.threadId'),c.native_id),
 coalesce(json_extract(v.value,'$.updatedAt'),json_extract(v.value,'$.createdAt'),json_extract(v.value,'$.completedAt'),json_extract(v.value,'$.requestedAt'),''),
 json_object('kind','subagent','id',json_extract(v.value,'$.id'),'conversation_id',coalesce(json_extract(v.value,'$.threadId'),c.native_id),
 'parent_id',NULL,'title',NULL,'run_id',json_extract(v.value,'$.runId'),'role',
 CASE WHEN json_extract(v.value,'$.role') IN ('user','assistant','system') THEN json_extract(v.value,'$.role') ELSE NULL END,
 'created_at',coalesce(json_extract(v.value,'$.createdAt'),json_extract(v.value,'$.requestedAt')),
 'updated_at',coalesce(json_extract(v.value,'$.updatedAt'),json_extract(v.value,'$.createdAt')),
 'completed',CASE WHEN json_extract(v.value,'$.completedAt') IS NOT NULL THEN json('true') ELSE json('false') END,
 'text',coalesce(json_extract(v.value,'$.text'),json_extract(v.value,'$.markdown'),json_extract(v.value,'$.input'),'') , 'native',json(v.value))
FROM history_conversations c,json_each(c.document,'$.subagents') v
WHERE c.source LIKE 't3:%' AND json_type(v.value,'$.id')='text';
INSERT OR IGNORE INTO history_projection_records
SELECT c.source,'turn_item',json_extract(v.value,'$.id'),coalesce(json_extract(v.value,'$.threadId'),c.native_id),
 coalesce(json_extract(v.value,'$.updatedAt'),json_extract(v.value,'$.createdAt'),json_extract(v.value,'$.completedAt'),json_extract(v.value,'$.requestedAt'),''),
 json_object('kind','turn_item','id',json_extract(v.value,'$.id'),'conversation_id',coalesce(json_extract(v.value,'$.threadId'),c.native_id),
 'parent_id',NULL,'title',NULL,'run_id',json_extract(v.value,'$.runId'),'role',
 CASE WHEN json_extract(v.value,'$.role') IN ('user','assistant','system') THEN json_extract(v.value,'$.role') ELSE NULL END,
 'created_at',coalesce(json_extract(v.value,'$.createdAt'),json_extract(v.value,'$.requestedAt')),
 'updated_at',coalesce(json_extract(v.value,'$.updatedAt'),json_extract(v.value,'$.createdAt')),
 'completed',CASE WHEN json_extract(v.value,'$.completedAt') IS NOT NULL THEN json('true') ELSE json('false') END,
 'text',coalesce(json_extract(v.value,'$.text'),json_extract(v.value,'$.markdown'),json_extract(v.value,'$.input'),'') , 'native',json(v.value))
FROM history_conversations c,json_each(c.document,'$.turnItems') v
WHERE c.source LIKE 't3:%' AND json_type(v.value,'$.id')='text';

PRAGMA user_version=8;
