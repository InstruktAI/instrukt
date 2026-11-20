//! A fake T3 Code host command: a `t3` script on the test's `PATH`, ahead
//! of any real one, answering `t3 thread start|send|show` the way the T3
//! server on the developer's computer does. instrukt's shipped `t3`
//! runtime entry drives it unchanged, so the entry's templates are what is
//! under test; a real `t3` is never called.
//!
//! Threads live in a directory of the test: `start` creates `thread:<n>`
//! in the folder it runs in (or a worktree of the requested branch),
//! `send` appends to a thread, and `show` prints a thread's state, as T3's
//! JSON with `--json`, or its latest messages. An unknown thread is "no
//! longer available", as T3 says.
#![allow(
    clippy::unwrap_used,
    dead_code,
    reason = "test setup: a failed step fails the test; each test crate uses what it needs"
)]

use std::os::unix::fs::PermissionsExt as _;
use std::path::PathBuf;

use crate::common::TestEnv;

const SCRIPT: &str = r###"#!/usr/bin/env python3
import hashlib, json, os, pathlib, sqlite3, subprocess, sys, time
root=pathlib.Path('__STATE__')
root.mkdir(parents=True,exist_ok=True)
args=sys.argv[1:]
with (root/'calls').open('a') as f: f.write(' '.join(args)+'\n')
if args==['--version']:
    print((root/'release').read_text() if (root/'release').exists() else 't3 v0.1.7'); sys.exit(0)
readiness=root.parent/'t3-readiness'
def clock_now():
    import datetime
    value=os.environ.get('INSTRUKT_NOW','')
    if value.startswith('@'): value=pathlib.Path(value[1:]).read_text().strip()
    if value: return datetime.datetime.fromisoformat(value.replace('Z','+00:00'))
    return datetime.datetime.now(datetime.timezone.utc)
if args[:3]==['auth','session','issue']:
    import base64, datetime, hashlib, time
    # A held issue waits until the test releases it; its pid tells whether
    # the launch is still running or was ended by its caller. A test that
    # failed before releasing it leaves no launch behind: it gives up after a
    # minute.
    if (root/'hold-issue').exists():
        held=root/'held-issues'; held.mkdir(exist_ok=True)
        (held/str(len(list(held.iterdir()))+1)).write_text(str(os.getpid()))
        given_up=time.monotonic()+60
        while not (root/'release-issue').exists():
            if time.monotonic()>given_up: sys.exit(1)
            time.sleep(0.02)
    if (root/'issue-failure').exists(): print('cannot issue a session',file=sys.stderr); sys.exit(1)
    # The next session number is reserved under the counter's lock, which
    # FakeT3::issued takes too: a late issue takes the next unused number and
    # never writes a lower one.
    import fcntl
    with open(root/'sessions.lock','a') as lock:
        fcntl.flock(lock,fcntl.LOCK_EX)
        counter=root/'sessions'; n=int(counter.read_text())+1 if counter.exists() else 1; counter.write_text(str(n))
    sid='00000000-0000-4000-8000-%012d'%n
    ttl=args[args.index('--ttl')+1]
    seconds=int(ttl[:-1])*{'s':1,'m':60,'h':3600}[ttl[-1]]
    scope=args[args.index('--scope')+1]
    claims={'v':1,'kind':'session','sid':sid,'method':'bearer-access-token','scopes':[scope]}
    token=base64.urlsafe_b64encode(json.dumps(claims).encode()).decode().rstrip('=')+'.test-signature'
    expires=(clock_now()+datetime.timedelta(seconds=seconds)).strftime('%Y-%m-%dT%H:%M:%SZ')
    (readiness/'tokens').mkdir(parents=True,exist_ok=True)
    (readiness/'tokens'/(sid+'.json')).write_text(json.dumps({'digest':hashlib.sha256(token.encode()).hexdigest(),'expires':expires,'revoked':False,'scope':scope}))
    (readiness/'clock-source').write_text(os.environ.get('INSTRUKT_NOW',''))
    print(token); sys.exit(0)
if args[:3]==['auth','session','revoke']:
    import time
    # A held revoke waits until the test releases it: every one while
    # `hold-revoke` is set, the next one once for `hold-next-revoke`. Its pid
    # tells whether it is still running; it gives up after a minute.
    hold=(root/'hold-revoke').exists()
    if (root/'hold-next-revoke').exists():
        (root/'hold-next-revoke').unlink(missing_ok=True); hold=True
    if hold:
        held=root/'held-revokes'; held.mkdir(exist_ok=True)
        (held/str(len(list(held.iterdir()))+1)).write_text(str(os.getpid()))
        given_up=time.monotonic()+60
        while not (root/'release-revoke').exists():
            if time.monotonic()>given_up: sys.exit(1)
            time.sleep(0.02)
    if (root/'revoke-failure').exists(): sys.exit(1)
    record=readiness/'tokens'/(args[-1]+'.json')
    if record.exists():
        session=json.loads(record.read_text()); session['revoked']=True; record.write_text(json.dumps(session))
    print('Revoked session.'); sys.exit(0)
if len(args)<2 or args[0]!='thread': sys.exit(2)
command=args[1]; args=args[2:]
flags={}; operands=[]
i=0
while i<len(args):
    if args[i] in ['--title','--worktree','--branch','--project','--messages','--provider','--model']:
        flags[args[i]]=args[i+1]; i+=2
    elif args[i]=='--': operands.extend(args[i+1:]); break
    elif args[i]=='--json': flags['--json']=True; i+=1
    else: operands.append(args[i]); i+=1
if (root/'down').exists() or (command=='list' and (root/'list-failure').exists()):
    print('connect ECONNREFUSED 127.0.0.1:3773',file=sys.stderr); sys.exit(1)
def all_threads():
    return [json.loads(p.read_text()) for p in sorted(root.glob('*.json'))]
def sync_inputs():
    db=pathlib.Path(os.environ['HOME'])/'.t3/userdata/statev2.sqlite'
    db.parent.mkdir(parents=True,exist_ok=True)
    with sqlite3.connect(db) as connection:
        connection.execute('CREATE TABLE IF NOT EXISTS orchestration_command_receipts (command_id TEXT PRIMARY KEY, aggregate_kind TEXT, aggregate_id TEXT, command_type TEXT, status TEXT)')
        connection.execute('CREATE TABLE IF NOT EXISTS orchestration_v2_projection_messages (message_id TEXT PRIMARY KEY,thread_id TEXT,role TEXT,payload_json TEXT)')
        connection.execute('CREATE TABLE IF NOT EXISTS orchestration_v2_projection_runs (run_id TEXT,thread_id TEXT,payload_json TEXT,PRIMARY KEY(thread_id,run_id))')
        def upsert(table, values):
            columns=[row[1] for row in connection.execute('PRAGMA table_info('+table+')') if row[1] in values]
            updates=','.join(column+'=excluded.'+column for column in columns)
            connection.execute('INSERT INTO '+table+' ('+','.join(columns)+') VALUES ('+','.join('?' for _ in columns)+') ON CONFLICT DO UPDATE SET '+updates+' WHERE payload_json!=excluded.payload_json',[values[column] for column in columns])
        # With `lifecycle`, a run is projected as T3 projects it: active while
        # it is the thread's running run, completed once it has ended.
        lifecycle=(root/'lifecycle').exists()
        # finish_turn supplies a separate completed run. This mode removes
        # the fake active projection when the corresponding native turn ends.
        separate_completion=(root/'separate-completion').exists()
        for thread in all_threads():
            runs=set()
            for index,message in enumerate(thread['messages']):
                if message['role']!='user': continue
                identity='fake:'+thread['threadId']+':input:'+str(index)
                run=message.get('runId')
                at='2026-11-06T10:00:00Z'
                payload=dict(message,id=identity,threadId=thread['threadId'],streaming=False,createdAt=at,updatedAt=at)
                upsert('orchestration_v2_projection_messages',dict(message_id=identity,thread_id=thread['threadId'],role='user',run_id=run,node_id=None,streaming=0,created_at=at,updated_at=at,payload_json=json.dumps(payload)))
                if run and run not in runs:
                    runs.add(run)
                    if separate_completion and (run!=thread.get('activeRunId') or thread['status']=='idle'):
                        connection.execute("DELETE FROM orchestration_v2_projection_runs WHERE thread_id=? AND run_id=?", (thread['threadId'],run))
                        continue
                    queued=message.get('queued',False)
                    entered=not queued and thread['status'] not in ['preparing','queued']
                    status='queued' if queued else (thread['status'] if thread['status'] in ['preparing','queued'] else 'running')
                    completed=None
                    if lifecycle and status=='running' and not (run==thread.get('activeRunId') and thread['status']=='running'):
                        status='completed'; completed=at
                    native={'id':run,'threadId':thread['threadId'],'userMessageId':identity,'status':status,'startedAt':at if entered else None,'requestedAt':at,'completedAt':completed}
                    if lifecycle: native['ordinal']=len(runs)
                    upsert('orchestration_v2_projection_runs',dict(run_id=run,thread_id=thread['threadId'],status=native['status'],requested_at=at,completed_at=completed,payload_json=json.dumps(native)))
        # With `hide-inputs`, a thread whose id the flag file names keeps no
        # input in the projection: its running turn's opening cannot be read.
        hidden=root/'hide-inputs'
        if hidden.exists():
            for thread in hidden.read_text().split():
                connection.execute("DELETE FROM orchestration_v2_projection_messages WHERE message_id LIKE ?",('fake:'+thread+':input:%',))
def save(t):
    temporary=root/(t['threadId']+'.'+str(os.getpid())+'.tmp')
    temporary.write_text(json.dumps(t)); temporary.replace(root/(t['threadId']+'.json')); sync_inputs()
def find(id):
    file=root/(id+'.json')
    if not file.exists(): print('Thread '+id+' is no longer available.',file=sys.stderr); sys.exit(1)
    return json.loads(file.read_text())
def project_for(checkout):
    path=str(pathlib.Path(checkout).resolve())
    default_id='project-'+hashlib.sha256(path.encode()).hexdigest()
    registration=root/(default_id+'.id')
    return registration.read_text() if registration.exists() else default_id
if command=='list':
    threads=all_threads()
    if '--project' in flags:
        project=project_for(flags['--project'])
        threads=[t for t in threads if (t.get('projectId') or project_for(t['checkout']))==project]
    if (root/'hide-project-id').exists():
        for t in threads: t.pop('projectId',None)
    if (root/'duplicate').exists() and threads:
        copy=dict(threads[0]); copy['threadId']='duplicate'; threads.append(copy)
    print(json.dumps([{k:v for k,v in t.items() if k not in ['messages','checkout','worktreePath','branch']} for t in threads]))
elif command=='start':
    watch=root/'watch-read-at-start'
    if watch.exists():
        read=readiness/'held-reads'/watch.read_text()
        (root/'read-at-start').write_text(read.read_text() if read.exists() else 'missing')
    while (root/'hold-start').exists():
        (root/'start-held').touch(); time.sleep(0.02)
    if (root/'start-failure').exists(): print('start failed',file=sys.stderr); sys.exit(1)
    if (root/'empty-start').exists(): sys.exit(0)
    if (root/'die-before-start').exists():
        (root/'die-before-start').unlink(); os.kill(os.getppid(),9); sys.exit(1)
    n=len(all_threads())+1
    id='s'+str(n)
    while (root/(id+'.json')).exists() or (root/(id+'.deleted')).exists():
        n+=1
        id='s'+str(n)
    named=root/'next-thread-id'
    if named.exists():
        id=named.read_text()
        named.unlink()
        assert not (root/(id+'.json')).exists() and not (root/(id+'.deleted')).exists()
    checkout=str(pathlib.Path(operands[0]).resolve())
    message=sys.stdin.read() if operands[-1]=='-' else operands[-1]
    t={'threadId':id,'projectId':project_for(checkout),'title':flags.get('--title','Developer thread'),'status':'running','latestRunId':'run:'+id+':1','activeRunId':'run:'+id+':1','updatedAt':'2026-11-06T10:00:00Z','link':'https://example.test/threads/'+id,'providerInstanceId':flags.get('--provider','codex'),'model':flags.get('--model','gpt-test'),'settled':False,'snoozed':False,'pendingRequestCount':0,'archived':False,'worktreePath':None,'branch':None,'checkout':checkout,'messages':[{'role':'user','text':message,'runId':'run:'+id+':1'}]}
    # With `queued-start`, the runtime accepts the thread's opening input
    # and keeps it queued: neither entered nor completed.
    if (root/'queued-start').exists(): t['status']='queued'
    if '--worktree' in flags:
        folder=root/'worktrees'/id
        folder.parent.mkdir(exist_ok=True)
        subprocess.run(['git','-C',checkout,'worktree','add','--quiet']+(['-b',flags['--branch']] if '--branch' in flags else [])+[str(folder),flags['--worktree']],check=True)
        t['worktreePath']=str(folder); t['branch']=flags.get('--branch',flags['--worktree']); t['base']=flags['--worktree']
    save(t)
    if (root/'die-after-start').exists():
        (root/'die-after-start').unlink(); os.kill(os.getppid(),9)
    elif not (root/'silent-start').exists(): print(id)
elif command=='send':
    text=sys.stdin.read() if operands[1]=='-' else operands[1]
    held=root/(operands[0]+'.hold-send')
    if held.exists():
        (root/(operands[0]+'.send-held')).write_text(str(os.getpid()))
    while held.exists(): time.sleep(0.02)
    t=find(operands[0])
    fail_once=root/(operands[0]+'.fail-next-send')
    if fail_once.exists():
        fail_once.unlink()
        print('No running T3 Code server uses the fixture store. Start T3 Code, then retry.',file=sys.stderr); sys.exit(1)
    ambiguous=root/(operands[0]+'.lose-next-send')
    if ambiguous.exists():
        ambiguous.unlink()
        if (root/(operands[0]+'.hold-lost-response')).exists():
            (root/(operands[0]+'.lost-response-held')).touch()
            while not (root/(operands[0]+'.release-lost-response')).exists(): time.sleep(0.02)
        print('send acknowledgment lost',file=sys.stderr); sys.exit(1)
    if (root/'send-failure').exists(): print('No running T3 Code server uses the fixture store. Start T3 Code, then retry.',file=sys.stderr); sys.exit(1)
    if (root/'send-unreachable').exists():
        print('Could not reach the T3 Code server at http://127.0.0.1:3773. connect ECONNREFUSED 127.0.0.1:3773\nAuthorization: Bearer diagnostic-test-secret\n'+text,file=sys.stderr); sys.exit(1)
    command_id=os.environ.get('T3_TEST_COMMAND_ID')
    reject=root/(operands[0]+'.reject-next-command')
    if command_id and reject.exists():
        reject.unlink()
        with sqlite3.connect(pathlib.Path(os.environ['HOME'])/'.t3/userdata/statev2.sqlite') as db:
            db.execute("INSERT INTO orchestration_command_receipts VALUES (?,'thread',?,'message.dispatch','rejected')", (command_id, operands[0]))
        print('send acknowledgment lost',file=sys.stderr); sys.exit(1)
    hold_submit=root/(operands[0]+'.hold-next-submit')
    held_submit=command_id and hold_submit.exists()
    if held_submit:
        hold_submit.unlink()
        (root/(operands[0]+'.submit-held-input')).write_text(text)
        (root/(operands[0]+'.submit-held')).write_text(command_id)
        while not (root/(operands[0]+'.release-submit')).exists(): time.sleep(0.02)
    if command_id:
        with (root/'dispatch-ids').open('a') as calls: calls.write(command_id+'\n')
        receipts=sqlite3.connect(root/'dispatch-receipts.sqlite')
        receipts.execute('CREATE TABLE IF NOT EXISTS command_receipts (command_id TEXT PRIMARY KEY)')
        receipts.execute('BEGIN IMMEDIATE')
        if receipts.execute('SELECT 1 FROM command_receipts WHERE command_id=?',(command_id,)).fetchone():
            if held_submit: (root/(operands[0]+'.late-deduplicated')).touch()
            sys.exit(0)
        t=find(operands[0])
    t['settled']=False; t['settledAt']=None
    queued=True
    run='run:'+t['threadId']+':'+str(len(t['messages'])+2)
    if t['status'] in ['idle','completed','stopped','failed','interrupted','cancelled','rolled_back']:
        queued=False; t['latestRunId']=run; t['activeRunId']=run; t['status']='running'
    t['messages'].append({'role':'user','text':text,'runId':run,'queued':queued,'busy':queued})
    defer=root/(operands[0]+'.defer-next-send')
    if defer.exists():
        defer.unlink()
        (root/(operands[0]+'.deferred-input')).write_text(json.dumps(t))
        print('send acknowledgment lost',file=sys.stderr); sys.exit(1)
    save(t)
    if command_id:
        receipts.execute('INSERT INTO command_receipts VALUES (?)',(command_id,))
        receipts.commit()
    if held_submit: (root/(operands[0]+'.late-submit')).touch()
    if (root/'die-after-send').exists():
        (root/'die-after-send').unlink()
        if command_id:
            os.kill(int((root/'runner-owner').read_text()),9)
        else: os.kill(os.getppid(),9)
    lost_ack=root/(t['threadId']+'.hold-show-after-send')
    if lost_ack.exists():
        lost_ack.unlink()
        (root/(t['threadId']+'.show-held')).unlink(missing_ok=True)
        (root/(t['threadId']+'.hold-show')).touch()
        print('send acknowledgment lost', file=sys.stderr); sys.exit(1)
elif command=='show':
    if (root/'show-failure').exists():
        print('native show unavailable', file=sys.stderr); sys.exit(1)
    held=root/(operands[0]+'.hold-show')
    if held.exists():
        try:
            with (root/(operands[0]+'.show-held')).open('x') as marker:
                marker.write(str(os.getpid()))
        except FileExistsError:
            pass
    while held.exists(): time.sleep(0.02)
    t=find(operands[0]); limit=int(flags.get('--messages',100))
    if limit or not os.environ.get('INSTRUKT_TEST_T3_WAKES'): sync_inputs()
    deferred=root/(operands[0]+'.deferred-input')
    missed=root/(operands[0]+'.deferred-input-missed')
    if limit==0 and deferred.exists():
        if missed.exists():
            ready=root/(operands[0]+'.deferred-ready-read')
            ready.write_text(str(os.getpid()))
            while not (root/(operands[0]+'.release-deferred-input')).exists(): time.sleep(0.02)
            # The readiness reply was read before the accepted input became
            # visible. Publish acceptance now and return that older reply.
            save(json.loads(deferred.read_text())); deferred.unlink(); missed.unlink()
        else:
            missed.touch()
            t=dict(t,status='running')

    if limit==0 and (root/'busy-before-entry').exists():
        counter=root/'entry-checks'; count=int(counter.read_text())+1 if counter.exists() else 1; counter.write_text(str(count))
        if count>=3:
            t['status']='running'; save(t); (root/'busy-before-entry').unlink(); counter.unlink()
    summary={k:v for k,v in t.items() if k not in ['messages','checkout'] and not (k=='projectId' and (root/'hide-project-id').exists())}
    if t['status'] in ['idle','completed','stopped','failed','interrupted','cancelled','rolled_back']:
        summary['activeRunId']=None
    print(json.dumps({'thread':summary,'messages':[{k:v for k,v in m.items() if k!='queued'} for m in [m for m in t['messages'] if not m.get('queued',False)][-limit:]] if limit else []}))
    if limit==0 and (root/'busy-after-idle-read').exists():
        counter=root/'asap-checks'; count=int(counter.read_text())+1 if counter.exists() else 1; counter.write_text(str(count))
        if count>=2:
            t['status']='running'; save(t); (root/'busy-after-idle-read').unlink(); counter.unlink()
else: sys.exit(2)
"###;

/// The fake `t3` of one test.
pub struct FakeT3 {
    script: PathBuf,
    state: PathBuf,
}

impl FakeT3 {
    pub fn release(&self, value: &str) {
        std::fs::write(self.state_dir().join("release"), value).unwrap();
    }

    pub fn state_dir(&self) -> &std::path::Path {
        &self.state
    }
    pub fn install(env: &mut TestEnv) -> Self {
        Self::create(env)
    }
    pub fn create(env: &TestEnv) -> Self {
        env.readiness_server();
        let bin = env.path("runtime-bin");
        let state = env.path("t3-state");
        std::fs::create_dir_all(&bin).unwrap();
        std::fs::create_dir_all(&state).unwrap();
        let script = bin.join("t3");
        std::fs::write(
            &script,
            SCRIPT.replace("__STATE__", &state.display().to_string()),
        )
        .unwrap();
        std::fs::set_permissions(&script, std::fs::Permissions::from_mode(0o755)).unwrap();
        Self { script, state }
    }
    fn default_project_id(checkout: &std::path::Path) -> String {
        use sha2::{Digest, Sha256};
        let root = checkout.canonicalize().unwrap();
        format!(
            "project-{}",
            hex::encode(Sha256::digest(root.to_string_lossy().as_bytes()))
        )
    }

    pub fn next_thread_id(&self, id: &str) {
        std::fs::write(self.state.join("next-thread-id"), id).unwrap();
    }

    pub fn register_project(&self, checkout: &std::path::Path, project: &str) {
        let id = Self::default_project_id(checkout);
        std::fs::write(self.state.join(format!("{id}.id")), project).unwrap();
    }

    pub fn project_id(&self, thread: &str) -> String {
        self.read(thread)["projectId"].as_str().unwrap().into()
    }

    pub fn seed(&self, id: &str, checkout: &std::path::Path, key: &str, brief: &str) {
        let default_id = Self::default_project_id(checkout);
        let project = std::fs::read_to_string(self.state.join(format!("{default_id}.id")))
            .unwrap_or(default_id);
        let thread = serde_json::json!({"threadId":id,"projectId":project,"checkout":checkout.to_string_lossy(),"title":key,"status":"running","latestRunId":format!("run:{id}:1"),"activeRunId":format!("run:{id}:1"),"updatedAt":"2026-11-06T10:00:00Z","pendingRequestCount":0,"archived":false,"settled":false,"worktreePath":null,"branch":null,"messages":[{"role":"user","text":brief,"runId":format!("run:{id}:1")}]});
        std::fs::write(
            self.state.join(format!("{id}.fixture")),
            serde_json::to_vec(&thread).unwrap(),
        )
        .unwrap();
        std::fs::rename(
            self.state.join(format!("{id}.fixture")),
            self.state.join(format!("{id}.json")),
        )
        .unwrap();
    }
    /// Projects the threads' current state into T3's store, as the T3 server
    /// does whenever a thread changes.
    pub fn project(&self, env: &TestEnv, id: &str) {
        let status = std::process::Command::new(&self.script)
            .args(["thread", "show", id, "--messages", "1"])
            .env("HOME", env.home())
            .stdout(std::process::Stdio::null())
            .status()
            .unwrap();
        assert!(status.success());
    }
    /// A person types `text` straight into the thread `id`, as T3's own
    /// interface sends it; with the thread idle it opens a turn of its own.
    pub fn person_sends(&self, env: &TestEnv, id: &str, text: &str) {
        let status = std::process::Command::new(&self.script)
            .args(["thread", "send", id, text])
            .env("HOME", env.home())
            .status()
            .unwrap();
        assert!(status.success());
    }
    pub fn input_log(&self, id: &str) -> String {
        self.read(id)["messages"]
            .as_array()
            .unwrap()
            .iter()
            .filter(|m| m["role"] == "user")
            .map(|m| m["text"].as_str().unwrap())
            .collect::<Vec<_>>()
            .join("\n\n")
    }
    pub fn threads(&self) -> Vec<String> {
        let mut ids: Vec<_> = std::fs::read_dir(&self.state)
            .unwrap()
            .map(|e| e.unwrap().path())
            .filter(|p| p.extension().is_some_and(|e| e == "json"))
            .map(|p| p.file_stem().unwrap().to_string_lossy().into_owned())
            .collect();
        ids.sort();
        ids
    }
    /// The semantic key a thread was started under, as its title carries
    /// it last.
    pub fn key(&self, id: &str) -> String {
        let title = self.read(id)["title"].as_str().unwrap().to_owned();
        title.rsplit(" · ").next().unwrap().to_owned()
    }
    /// The provider instance a thread was started on.
    pub fn provider(&self, id: &str) -> instrukt_core::ids::ProviderInstanceId {
        instrukt_core::ids::ProviderInstanceId::parse(
            self.read(id)["providerInstanceId"].as_str().unwrap(),
        )
        .unwrap()
    }
    fn read(&self, id: &str) -> serde_json::Value {
        serde_json::from_slice(&std::fs::read(self.state.join(format!("{id}.json"))).unwrap())
            .unwrap()
    }
    pub fn change(&self, id: &str, key: &str, value: serde_json::Value) {
        let mut t = self.read(id);
        t[key] = value;
        std::fs::write(
            self.state.join(format!("{id}.fixture")),
            serde_json::to_vec(&t).unwrap(),
        )
        .unwrap();
        std::fs::rename(
            self.state.join(format!("{id}.fixture")),
            self.state.join(format!("{id}.json")),
        )
        .unwrap();
    }
    pub fn flag(&self, name: &str, on: bool) {
        let p = self.state.join(name);
        if on {
            std::fs::write(p, "").unwrap();
        } else if p.exists() {
            std::fs::remove_file(p).unwrap();
        }
    }
    /// Whether a start has been held by the `hold-start` flag.
    pub fn start_held(&self) -> bool {
        self.state.join("start-held").exists()
    }
    /// The run thread `id` has active, as the fake records it.
    pub fn active_run(&self, id: &str) -> String {
        self.read(id)["activeRunId"].as_str().unwrap().to_owned()
    }
    /// Thread `id`'s inputs leave T3's projection and stay out until
    /// [`FakeT3::show_inputs`].
    pub fn hide_inputs(&self, id: &str) {
        std::fs::write(self.state.join("hide-inputs"), id).unwrap();
    }
    pub fn show_inputs(&self) {
        self.flag("hide-inputs", false);
    }
    pub fn set_status(&self, id: &str, value: &str) {
        if matches!(value, "idle" | "completed" | "stopped" | "failed") {
            self.consume(id);
        }
        self.change(id, "status", value.into());
    }
    pub fn consume(&self, id: &str) {
        let mut t = self.read(id);
        let pending = t["messages"]
            .as_array()
            .unwrap()
            .iter()
            .any(|m| m["role"] == "user" && m["queued"] == true);
        if pending {
            let mut entered = None;
            for message in t["messages"].as_array_mut().unwrap() {
                if message["role"] == "user" && message["queued"] == true {
                    message["queued"] = false.into();
                    entered = Some(message["runId"].clone());
                }
            }
            if let Some(run) = entered {
                t["latestRunId"] = run.clone();
                t["activeRunId"] = run;
            }
        }
        std::fs::write(
            self.state.join(format!("{id}.fixture")),
            serde_json::to_vec(&t).unwrap(),
        )
        .unwrap();
        std::fs::rename(
            self.state.join(format!("{id}.fixture")),
            self.state.join(format!("{id}.json")),
        )
        .unwrap();
    }
    pub fn finish(&self, id: &str, status: &str, text: &str) {
        self.set_status(id, status);
        let mut t = self.read(id);
        let run = t["latestRunId"].clone();
        t["messages"]
            .as_array_mut()
            .unwrap()
            .push(serde_json::json!({"role":"assistant","text":text,"runId":run}));
        std::fs::write(
            self.state.join(format!("{id}.fixture")),
            serde_json::to_vec(&t).unwrap(),
        )
        .unwrap();
        std::fs::rename(
            self.state.join(format!("{id}.fixture")),
            self.state.join(format!("{id}.json")),
        )
        .unwrap();
    }
    pub fn set_pending(&self, id: &str, count: u32) {
        self.change(id, "pendingRequestCount", count.into());
    }
    pub fn delete(&self, id: &str) {
        std::fs::rename(
            self.state.join(format!("{id}.json")),
            self.state.join(format!("{id}.deleted")),
        )
        .unwrap();
    }
    pub fn archive(&self, id: &str) {
        self.change(id, "archived", true.into());
    }
    /// The sessions issued so far, in order: read under the lock the fake's
    /// issue takes to reserve each number, so the count never goes back.
    pub fn issued(&self) -> usize {
        let lock = std::fs::OpenOptions::new()
            .create(true)
            .append(true)
            .open(self.state.join("sessions.lock"))
            .unwrap();
        rustix::fs::flock(&lock, rustix::fs::FlockOperation::LockShared).unwrap();
        let issued = std::fs::read_to_string(self.state.join("sessions"))
            .map_or(0, |n| n.trim().parse().unwrap());
        drop(lock);
        issued
    }
    /// How many `auth session revoke` launches were held so far.
    pub fn held_revokes(&self) -> usize {
        std::fs::read_dir(self.state.join("held-revokes")).map_or(0, Iterator::count)
    }
    /// Whether the `n`th held `auth session revoke` launch (from 1) is still
    /// running.
    pub fn held_revoke_running(&self, n: usize) -> bool {
        self.running(&format!("held-revokes/{n}"))
    }
    /// How many times `auth session revoke` was launched for `session`.
    pub fn revoke_attempts(&self, session: &str) -> usize {
        let call = format!("auth session revoke -- {session}");
        self.calls().iter().filter(|line| **line == call).count()
    }
    /// Whether the process whose pid the state file `record` holds is
    /// still running.
    fn running(&self, record: &str) -> bool {
        let Ok(pid) = std::fs::read_to_string(self.state.join(record)) else {
            return false;
        };
        std::process::Command::new("kill")
            .args(["-0", pid.trim()])
            .stderr(std::process::Stdio::null())
            .status()
            .unwrap()
            .success()
    }
    pub fn held_send_running(&self, session: &str) -> bool {
        self.running(&format!("{session}.send-held"))
    }
    /// Lose this target's next send acknowledgment and hold its read-back.
    pub fn hold_show_after_send(&self, session: &str) {
        self.flag(&format!("{session}.hold-show-after-send"), true);
    }
    /// The original held show is still running, even if another was launched.
    pub fn held_show_running(&self, session: &str) -> bool {
        self.running(&format!("{session}.show-held"))
    }
    /// How many `auth session issue` launches were held so far.
    pub fn held_issues(&self) -> usize {
        std::fs::read_dir(self.state.join("held-issues")).map_or(0, Iterator::count)
    }
    /// Whether the `n`th held `auth session issue` launch (from 1) is still
    /// running: its process alive, neither ended by its caller nor released.
    pub fn held_issue_running(&self, n: usize) -> bool {
        let Ok(pid) = std::fs::read_to_string(self.state.join(format!("held-issues/{n}"))) else {
            return false;
        };
        std::process::Command::new("kill")
            .args(["-0", pid.trim()])
            .stderr(std::process::Stdio::null())
            .status()
            .unwrap()
            .success()
    }
    pub fn server_down(&self) {
        self.flag("down", true);
    }
    pub fn folder(&self, id: &str) -> PathBuf {
        self.read(id)["checkout"].as_str().unwrap().into()
    }
    pub fn worktree(&self, id: &str) -> PathBuf {
        self.read(id)["worktreePath"].as_str().unwrap().into()
    }
    pub fn worktree_base(&self, id: &str) -> String {
        self.read(id)["base"].as_str().unwrap().into()
    }
    pub fn title(&self, id: &str) -> String {
        self.read(id)["title"].as_str().unwrap().into()
    }
    pub fn message_count(&self, id: &str) -> usize {
        self.read(id)["messages"].as_array().unwrap().len()
    }
    pub fn messages(&self, id: &str) -> String {
        serde_json::to_string(&self.read(id)["messages"]).unwrap()
    }
    pub fn message(&self, id: &str, n: usize) -> String {
        self.read(id)["messages"][n]["text"]
            .as_str()
            .unwrap()
            .into()
    }
    pub fn calls(&self) -> Vec<String> {
        std::fs::read_to_string(self.state.join("calls"))
            .unwrap_or_default()
            .lines()
            .map(str::to_owned)
            .collect()
    }
    pub fn developer_starts_in(&self, checkout: &std::path::Path) -> String {
        let out = std::process::Command::new(&self.script)
            .args([
                "thread",
                "start",
                checkout.to_str().unwrap(),
                "A developer conversation.",
            ])
            .output()
            .unwrap();
        assert!(out.status.success());
        String::from_utf8(out.stdout).unwrap().trim().into()
    }
    pub fn developer_starts_thread(&self) -> String {
        let out = std::process::Command::new(&self.script)
            .args([
                "thread",
                "start",
                self.state.to_str().unwrap(),
                "Look into the flaky test.",
            ])
            .output()
            .unwrap();
        assert!(out.status.success());
        String::from_utf8(out.stdout).unwrap().trim().into()
    }
}
