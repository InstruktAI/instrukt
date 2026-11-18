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
import json, os, pathlib, sqlite3, subprocess, sys
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
        (root/'held-issue').write_text(str(os.getpid()))
        given_up=time.monotonic()+60
        while not (root/'release-issue').exists():
            if time.monotonic()>given_up: sys.exit(1)
            time.sleep(0.02)
    if (root/'issue-failure').exists(): print('cannot issue a session',file=sys.stderr); sys.exit(1)
    counter=root/'sessions'; n=int(counter.read_text())+1 if counter.exists() else 1; counter.write_text(str(n))
    sid='00000000-0000-4000-8000-%012d'%n
    ttl=args[args.index('--ttl')+1]
    seconds=int(ttl[:-1])*{'s':1,'m':60,'h':3600}[ttl[-1]]
    claims={'v':1,'kind':'session','sid':sid,'method':'bearer-access-token','scopes':['orchestration:read']}
    token=base64.urlsafe_b64encode(json.dumps(claims).encode()).decode().rstrip('=')+'.test-signature'
    expires=(clock_now()+datetime.timedelta(seconds=seconds)).strftime('%Y-%m-%dT%H:%M:%SZ')
    (readiness/'tokens').mkdir(parents=True,exist_ok=True)
    (readiness/'tokens'/(sid+'.json')).write_text(json.dumps({'digest':hashlib.sha256(token.encode()).hexdigest(),'expires':expires,'revoked':False}))
    (readiness/'clock-source').write_text(os.environ.get('INSTRUKT_NOW',''))
    print(token); sys.exit(0)
if args[:3]==['auth','session','revoke']:
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
        connection.execute('CREATE TABLE IF NOT EXISTS orchestration_v2_projection_messages (message_id TEXT PRIMARY KEY,thread_id TEXT,role TEXT,payload_json TEXT)')
        connection.execute('CREATE TABLE IF NOT EXISTS orchestration_v2_projection_runs (run_id TEXT,thread_id TEXT,payload_json TEXT,PRIMARY KEY(thread_id,run_id))')
        def upsert(table, values):
            columns=[row[1] for row in connection.execute('PRAGMA table_info('+table+')') if row[1] in values]
            updates=','.join(column+'=excluded.'+column for column in columns)
            connection.execute('INSERT INTO '+table+' ('+','.join(columns)+') VALUES ('+','.join('?' for _ in columns)+') ON CONFLICT DO UPDATE SET '+updates+' WHERE payload_json!=excluded.payload_json',[values[column] for column in columns])
        # With `lifecycle`, a run is projected as T3 projects it: active while
        # it is the thread's running run, completed once it has ended.
        lifecycle=(root/'lifecycle').exists()
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
                    queued=message.get('queued',False)
                    entered=not queued and thread['status'] not in ['preparing','queued']
                    status='queued' if queued else (thread['status'] if thread['status'] in ['preparing','queued'] else 'running')
                    completed=None
                    if lifecycle and status=='running' and not (run==thread.get('activeRunId') and thread['status']=='running'):
                        status='completed'; completed=at
                    native={'id':run,'threadId':thread['threadId'],'userMessageId':identity,'status':status,'startedAt':at if entered else None,'requestedAt':at,'completedAt':completed}
                    if lifecycle: native['ordinal']=len(runs)
                    upsert('orchestration_v2_projection_runs',dict(run_id=run,thread_id=thread['threadId'],status=native['status'],requested_at=at,completed_at=completed,payload_json=json.dumps(native)))
def save(t):
    temporary=root/(t['threadId']+'.'+str(os.getpid())+'.tmp')
    temporary.write_text(json.dumps(t)); temporary.replace(root/(t['threadId']+'.json')); sync_inputs()
def find(id):
    file=root/(id+'.json')
    if not file.exists(): print('Thread '+id+' is no longer available.',file=sys.stderr); sys.exit(1)
    return json.loads(file.read_text())
if command=='list':
    threads=all_threads()
    if '--project' in flags: threads=[t for t in threads if t['checkout']==flags['--project']]
    if (root/'hide-project-id').exists():
        for t in threads: t.pop('projectId',None)
    if (root/'duplicate').exists() and threads:
        copy=dict(threads[0]); copy['threadId']='duplicate'; threads.append(copy)
    print(json.dumps([{k:v for k,v in t.items() if k not in ['messages','checkout','worktreePath','branch']} for t in threads]))
elif command=='start':
    if (root/'empty-start').exists(): sys.exit(0)
    if (root/'die-before-start').exists():
        (root/'die-before-start').unlink(); os.kill(os.getppid(),9); sys.exit(1)
    n=len(all_threads())+1
    id='s'+str(n)
    checkout=str(pathlib.Path(operands[0]).resolve())
    message=sys.stdin.read() if operands[-1]=='-' else operands[-1]
    t={'threadId':id,'projectId':checkout,'title':flags.get('--title','Developer thread'),'status':'running','latestRunId':'run:'+id+':1','activeRunId':'run:'+id+':1','updatedAt':'2026-11-06T10:00:00Z','link':'https://example.test/threads/'+id,'providerInstanceId':flags.get('--provider','codex'),'model':flags.get('--model','gpt-test'),'settled':False,'snoozed':False,'pendingRequestCount':0,'archived':False,'worktreePath':None,'branch':None,'checkout':checkout,'messages':[{'role':'user','text':message,'runId':'run:'+id+':1'}]}
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
    t=find(operands[0])
    if (root/'send-failure').exists(): print('send failed',file=sys.stderr); sys.exit(1)
    if (root/'send-unreachable').exists():
        print('Could not reach the T3 Code server at http://127.0.0.1:3773. connect ECONNREFUSED 127.0.0.1:3773\nAuthorization: Bearer diagnostic-test-secret\n'+sys.stdin.read(),file=sys.stderr); sys.exit(1)
    text=sys.stdin.read() if operands[1]=='-' else operands[1]
    t['settled']=False; t['settledAt']=None
    queued=True
    run='run:'+t['threadId']+':'+str(len(t['messages'])+2)
    if t['status'] in ['idle','completed','stopped','failed','interrupted','cancelled','rolled_back']:
        queued=False; t['latestRunId']=run; t['activeRunId']=run; t['status']='running'
    t['messages'].append({'role':'user','text':text,'runId':run,'queued':queued,'busy':queued}); save(t)
    if (root/'die-after-send').exists():
        (root/'die-after-send').unlink(); os.kill(os.getppid(),9)
elif command=='show':
    sync_inputs()
    t=find(operands[0]); limit=int(flags.get('--messages',100))
    if limit==0 and (root/'busy-before-entry').exists():
        counter=root/'entry-checks'; count=int(counter.read_text())+1 if counter.exists() else 1; counter.write_text(str(count))
        if count>=3:
            t['status']='running'; save(t); (root/'busy-before-entry').unlink(); counter.unlink()
    print(json.dumps({'thread':{k:v for k,v in t.items() if k not in ['messages','checkout'] and not (k=='projectId' and (root/'hide-project-id').exists())},'messages':[{k:v for k,v in m.items() if k!='queued'} for m in [m for m in t['messages'] if not m.get('queued',False)][-limit:]] if limit else []}))
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
    pub fn seed(&self, id: &str, checkout: &std::path::Path, key: &str, brief: &str) {
        let thread = serde_json::json!({"threadId":id,"projectId":checkout.to_string_lossy(),"checkout":checkout.to_string_lossy(),"title":key,"status":"running","latestRunId":format!("run:{id}:1"),"activeRunId":format!("run:{id}:1"),"updatedAt":"2026-11-06T10:00:00Z","pendingRequestCount":0,"archived":false,"settled":false,"worktreePath":null,"branch":null,"messages":[{"role":"user","text":brief,"runId":format!("run:{id}:1")}]});
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
    pub fn archive(&self, id: &str) {
        self.change(id, "archived", true.into());
    }
    /// The sessions issued so far, in order.
    pub fn issued(&self) -> usize {
        std::fs::read_to_string(self.state.join("sessions"))
            .map_or(0, |n| n.trim().parse().unwrap())
    }
    /// Whether the held `auth session issue` launch has arrived and is still
    /// running: its process alive, neither ended by its caller nor released.
    pub fn held_issue_running(&self) -> bool {
        let Ok(pid) = std::fs::read_to_string(self.state.join("held-issue")) else {
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
