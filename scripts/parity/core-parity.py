"""Core parity run: instrukt, publishing the predecessor's own domains, against
the predecessor's `telec docs get` and `telec docs index`.

A one-off acceptance run (docs/local/policy/testing.md), not a CI test.

    python3 -I scripts/parity/core-parity.py \
        --predecessor /path/to/TeleClaude --instrukt target/debug/instrukt \
        --out /some/dir/outside/the/repository [--keep]

What it does:

1. Copies the predecessor's committed `docs/` (``git archive HEAD docs``) into a
   fresh repository under a temporary directory, declares project
   ``teleclaude`` publishing the domains the predecessor's ``teleclaude.yml``
   publishes, commits, and runs ``instrukt sync`` with ``HOME`` set to a
   temporary directory. Nothing of the predecessor is written.
2. Takes the ids from the predecessor's own ``telec docs index`` (run in the
   predecessor's checkout, read-only), restricted to what that checkout
   publishes: its domains, its local snippets and its third-party docs.
3. For every id compares ``instrukt docs get <id>`` with ``telec docs get
   <id>``; for every activatable procedure the ``--closure`` reads; and the
   index listing unfiltered, per domain, per area, ``--baseline-only`` and
   ``--third-party``.

Only what the specs say legitimately differs is normalised (each rule names
its spec in ``NORMALISATIONS``). Everything else is reported as a difference.
The output directory receives one file per difference and ``summary.md`` /
``summary.json``. The temporary home and repository are removed unless
``--keep`` is given.
"""

from __future__ import annotations

import argparse
import io
import json
import os
import re
import shutil
import subprocess
import sys
import tarfile
import tempfile
from pathlib import Path

TAXONOMIES = ["principle", "concept", "policy", "procedure", "design", "spec", "adr"]
ROOT_KEYS = ("activation_trigger",)


def run(cmd, cwd, env=None, stdin=None):
    proc = subprocess.run(
        cmd, cwd=cwd, env=env, input=stdin, capture_output=True, timeout=120
    )
    return proc.returncode, proc.stdout.decode("utf-8", "replace"), proc.stderr.decode(
        "utf-8", "replace"
    )


def publishes(config_text: str) -> list[tuple[str, str]]:
    """The `business.publishes` slugs and triggers of the predecessor's
    teleclaude.yml (folded scalars joined with spaces)."""
    out: list[tuple[str, str]] = []
    lines = config_text.splitlines()
    start = next(i for i, l in enumerate(lines) if l.strip() == "publishes:")
    indent = len(lines[start]) - len(lines[start].lstrip())
    slug = trigger = None
    for line in lines[start + 1 :]:
        if line.strip() and len(line) - len(line.lstrip()) <= indent:
            break
        text = line.strip()
        if text.startswith("- slug:"):
            if slug:
                out.append((slug, trigger or ""))
            slug, trigger = text.split(":", 1)[1].strip(), None
        elif text.startswith("trigger:"):
            trigger = text.split(":", 1)[1].strip().strip("'\"")
        elif trigger is not None and text:
            trigger = f"{trigger} {text.strip(chr(39) + chr(34))}"
    if slug:
        out.append((slug, trigger or ""))
    return out


def frontmatter(text: str) -> str:
    if not text.startswith("---\n"):
        return ""
    end = text.find("\n---\n", 4)
    return text[4:end] if end > 0 else ""


def entry_lines(text: str) -> dict[str, str]:
    """`<id>: <description>[ suffixes]` lines of an index listing, by id."""
    out = {}
    for line in text.splitlines():
        if not line or line.startswith("#"):
            continue
        m = re.match(r"^([a-z0-9][^\s:]*): ?(.*)$", line)
        if m:
            out[m.group(1)] = re.sub(r"\s+", " ", m.group(2)).strip()
    return out


def header_lines(text: str) -> list[str]:
    return [l for l in text.splitlines() if l.startswith("#")]


def parse_get(text: str) -> dict:
    """A `docs get` reading: its header comment lines, the required reads it
    names (not loaded or included) and its blocks, each with its header
    fields, body and trailing notice."""
    lines = text.splitlines()
    blocks = []
    header = []
    i = 0
    while i < len(lines) and not (
        lines[i] == "---" and i + 1 < len(lines) and re.match(r"^(id|bundle): ", lines[i + 1])
    ):
        header.append(lines[i])
        i += 1
    while i < len(lines):
        # frontmatter of one block
        i += 1
        fields = {}
        while i < len(lines) and lines[i] != "---":
            if ": " in lines[i]:
                k, v = lines[i].split(": ", 1)
                fields[k] = v
            i += 1
        i += 1
        body = []
        while i < len(lines) and not (
            lines[i] == "---"
            and i + 1 < len(lines)
            and re.match(r"^(id|bundle): ", lines[i + 1])
        ):
            body.append(lines[i])
            i += 1
        # a trailing notice: the last run of `# ` lines that starts `# NOTE`
        notice = []
        j = len(body)
        while j > 0 and (body[j - 1].startswith("# ") or not body[j - 1].strip()):
            j -= 1
        tail = body[j:]
        if any(l.startswith("# NOTE") for l in tail):
            notice = [l for l in tail if l.strip()]
            body = body[:j]
        else:
            k = next((n for n, l in enumerate(tail) if l.startswith("# NOTE")), None)
            if k is not None:
                notice = tail[k:]
                body = body[: j + k]
        blocks.append(
            {
                "fields": fields,
                "body": "\n".join(l.rstrip() for l in body).strip("\n"),
                "notice": [l.rstrip() for l in notice],
            }
        )
    reads = {"not_loaded": [], "included": []}
    for line in header:
        m = re.search(r"Required reads \((not loaded|included)\)[^:]*: (.*)$", line)
        if m:
            key = "not_loaded" if m.group(1) == "not loaded" else "included"
            reads[key] = [r.strip() for r in m.group(2).split(",") if r.strip()]
    return {"header": header, "reads": reads, "blocks": blocks}


class Normaliser:
    """Maps both tools' outputs onto one vocabulary, only where a spec says
    they legitimately differ."""

    def __init__(self, pred_repo: Path, pred_home: Path, clone: Path, home: Path):
        self.rules = [
            # progressive-retrieval § Read pipeline (inline references become
            # absolute paths) + architecture overview § The home: the
            # published tree is ~/.instrukt/docs, the predecessor's is
            # ~/.teleclaude/docs.
            (str(home / ".instrukt" / "docs"), "<HOME_DOCS>"),
            (str(pred_home / ".teleclaude" / "docs"), "<HOME_DOCS>"),
            ("~/.instrukt/docs", "<HOME_DOCS>"),
            ("~/.teleclaude/docs", "<HOME_DOCS>"),
            # snippet-authoring-schema § Directory layout: a local entry
            # carries the absolute path of the file in its project; the
            # project is checked out in a different place in each run.
            (str(clone), "<REPO>"),
            (str(pred_repo), "<REPO>"),
        ]

    def __call__(self, text: str) -> str:
        for old, new in self.rules:
            text = text.replace(old, new)
        # docs domain spec § Published contracts: the commands are
        # `instrukt docs …`; the predecessor names `telec docs …`.
        text = re.sub(r"\btelec\b", "instrukt", text)
        return text


NORMALISATIONS = [
    "home docs path ~/.teleclaude/docs = ~/.instrukt/docs "
    "(architecture overview § The home; progressive-retrieval § Read pipeline, inline references)",
    "project checkout path = clone path "
    "(snippet-authoring-schema § Directory layout: local entries carry their project's absolute path)",
    "tool name telec = instrukt (docs domain spec § Published contracts)",
]


#: The machine's runtime mapping in the temporary home: one entry per agent
#: the predecessor's `dispatch` declarations name, so they validate
#: (snippet-authoring-schema § dispatch: agent names are those the configured
#: runtime mapping can start). Machine configuration, not predecessor content.
MACHINE = "schema_version: 1\nruntimes:\n" + "".join(
    f"  - name: {a}-cli\n    agent: {a}\n    session_id_env: PARITY_{a.upper()}_SESSION\n"
    f"    start: {a}\n    send: {a} send\n    alive: {a} alive\n    conversation: {a} history\n"
    for a in ("claude", "codex", "deepseek", "gemini")
)

# The source migrations
# ---------------------
#
# The predecessor's committed docs do not meet instrukt's source contract in
# the ways below. The owner has ruled that instrukt is not loosened to accept
# any of them: these migrations are the reference for the predecessor's
# cut-over commit, which applies them to its own sources. The run applies
# them to its copy only, and records every file each one touches in
# `migrations.json`.
#
# - home-reference-form: required reads are committed home-expanded as
#   `@/Users/<user>/.teleclaude/docs/<id>.md`. instrukt's reference form is
#   `~/.instrukt/docs/<id>.md` (snippet-authoring-schema, reference forms;
#   architecture overview § The home). Rewritten to that form.
# - adr-number-integer: ADR `number` authored as a quoted string
#   (`number: '1'`). The schema requires an integer. The quotes are dropped;
#   frontmatter only, never rendered. (Unquoted numbers with leading zeros,
#   `number: 0049`, are valid and left alone.)
# - third-party-classification-removed: third-party docs authoring `id`,
#   `type` or `scope`. Third-party docs author only `description`
#   (third-party-docs spec). The keys are removed; frontmatter only.
# - authored-classification-removed: snippets authoring `id`, `type`, `scope`
#   and `domain`, which are derived from the path, never authored
#   (snippet-authoring-schema). The keys are removed; frontmatter only.
# - quarantined: files outside instrukt's documentation model, left out of
#   the copy: `docs/explore/`, `docs/prompts/` and `docs/snippets/` (no such
#   halves; fragments live in `docs/global/snippets/`), and
#   `docs/local/baseline-progressive.md` (a project has only
#   `docs/local/baseline.md`).

#: Directories and files outside instrukt's documentation model.
QUARANTINE_DIRS = ("docs/explore", "docs/prompts", "docs/snippets")
QUARANTINE_FILES = ("docs/local/baseline-progressive.md",)


def migrate(clone: Path, user_home: Path) -> dict:
    """Applies the source migrations above to the copy. Returns the files
    each migration touched."""
    done: dict[str, list[str]] = {
        "home-reference-form": [],
        "adr-number-integer": [],
        "third-party-classification-removed": [],
        "authored-classification-removed": [],
        "quarantined": [],
    }
    for rel in QUARANTINE_DIRS:
        if (clone / rel).exists():
            done["quarantined"] += [str(p.relative_to(clone)) for p in (clone / rel).rglob("*.md")]
            shutil.rmtree(clone / rel)
    for rel in QUARANTINE_FILES:
        if (clone / rel).exists():
            (clone / rel).unlink()
            done["quarantined"].append(rel)
    old_home = f"{user_home}/.teleclaude/docs/"
    for path in sorted((clone / "docs").rglob("*.md")):
        rel = str(path.relative_to(clone))
        text = path.read_text()
        new = text
        # architecture overview § The home: published references name
        # ~/.instrukt/docs; the predecessor commits ~/.teleclaude/docs,
        # home-expanded.
        new = new.replace(old_home, "~/.instrukt/docs/").replace("~/.teleclaude/docs/", "~/.instrukt/docs/")
        if new != text:
            done["home-reference-form"].append(rel)
        fm = frontmatter(new)
        if fm:
            fm2 = fm
            if "/adr/" in rel:
                fm2 = re.sub(r"^number: (['\"])0*(\d+)\1$", r"number: \2", fm2, flags=re.M)
                if fm2 != fm:
                    done["adr-number-integer"].append(rel)
            keys = ("id", "type", "scope", "domain")
            present = [k for k in keys if re.search(rf"^{k}:", fm2, re.M)]
            if present and (rel.startswith("docs/third-party/") or len(present) == len(keys)):
                fm3 = "\n".join(l for l in fm2.split("\n") if not re.match(r"^(id|type|scope|domain):", l))
                done[
                    "third-party-classification-removed"
                    if rel.startswith("docs/third-party/")
                    else "authored-classification-removed"
                ].append(rel)
                fm2 = fm3
            new = new.replace(f"---\n{fm}\n---\n", f"---\n{fm2}\n---\n", 1)
        if new != text:
            path.write_text(new)
    return done


def main() -> int:
    ap = argparse.ArgumentParser()
    ap.add_argument("--predecessor", required=True, type=Path)
    ap.add_argument("--instrukt", required=True, type=Path)
    ap.add_argument("--out", required=True, type=Path)
    ap.add_argument("--keep", action="store_true")
    args = ap.parse_args()
    pred = args.predecessor.resolve()
    binary = args.instrukt.resolve()
    out = args.out.resolve()
    out.mkdir(parents=True, exist_ok=True)
    work = Path(tempfile.mkdtemp(prefix="instrukt-parity-")).resolve()
    try:
        return parity(pred, binary, out, work)
    finally:
        if args.keep:
            print(f"kept {work}", file=sys.stderr)
        else:
            shutil.rmtree(work, ignore_errors=True)


def parity(pred: Path, binary: Path, out: Path, work: Path) -> int:
    home = work / "home"
    home.mkdir()
    (home / ".gitconfig").write_text(
        "[user]\n\tname = Parity Run\n\temail = parity@example.test\n"
        "[init]\n\tdefaultBranch = main\n[commit]\n\tgpgsign = false\n"
    )
    env = {
        "PATH": os.environ.get("PATH", ""),
        "HOME": str(home),
        "GIT_CONFIG_GLOBAL": str(home / ".gitconfig"),
        "GIT_CONFIG_NOSYSTEM": "1",
        "LANG": "C.UTF-8",
    }
    clone = work / "teleclaude"
    clone.mkdir()
    code, pred_head, _ = run(["git", "rev-parse", "HEAD"], pred)
    archive = subprocess.run(
        ["git", "archive", "--format=tar", "HEAD", "docs"],
        cwd=pred,
        capture_output=True,
        check=True,
    ).stdout
    with tarfile.open(fileobj=io.BytesIO(archive)) as tar:
        tar.extractall(clone, filter="data")
    migrated = migrate(clone, Path(os.environ["HOME"]))
    (out / "migrations.json").write_text(json.dumps(migrated, indent=2))
    (home / ".instrukt").mkdir()
    (home / ".instrukt" / "instrukt.yaml").write_text(MACHINE)
    domains = publishes((pred / "teleclaude.yml").read_text())
    layer = ["schema_version: 1", "project: teleclaude", "publishes:"]
    for slug, trigger in domains:
        layer.append(f"  - slug: {slug}")
        layer.append(f"    trigger: {json.dumps(trigger)}")
    (clone / "instrukt.yaml").write_text("\n".join(layer) + "\n")
    for cmd in (
        ["git", "init", "--quiet", "--initial-branch=main"],
        ["git", "add", "--all"],
        ["git", "commit", "--quiet", "-m", f"parity: predecessor docs at {pred_head.strip()}"],
    ):
        code, _, err = run(cmd, clone, env)
        if code:
            raise SystemExit(f"{cmd}: {err}")
    code, so, se = run([str(binary), "sync"], clone, env)
    (out / "sync.txt").write_text(f"exit {code}\n--- stdout\n{so}\n--- stderr\n{se}")
    if code:
        print(f"instrukt sync failed (exit {code}); see {out / 'sync.txt'}", file=sys.stderr)
        return 2

    norm = Normaliser(pred, Path(os.environ["HOME"]), clone, home)
    names = [d for d, _ in domains]

    def telec(args):
        return run(["telec", *args], pred)

    def instrukt(args):
        return run([str(binary), *args], clone, env)

    # The ids: what the predecessor's own index lists, restricted to what
    # its checkout publishes.
    _, ref_index, _ = telec(["docs", "index", "--third-party"])
    third_party = {
        "third-party/" + str(p.relative_to(clone / "docs" / "third-party"))[:-3]
        for p in (clone / "docs" / "third-party").rglob("*.md")
    }

    def ours(i):
        return i.split("/")[0] in names or i.startswith("local/projects/teleclaude/") or i in third_party

    ids = sorted(i for i in entry_lines(ref_index) if ours(i))

    def source(i):
        if i.startswith("local/projects/teleclaude/"):
            return clone / "docs" / "local" / (i.removeprefix("local/projects/teleclaude/") + ".md")
        return clone / "docs" / "global" / (i + ".md")

    closures = []
    for i in ids:
        parts = i.split("/")
        tax = parts[3] if i.startswith("local/") else parts[1]
        if tax != "procedure" or not source(i).is_file():
            continue
        fm = frontmatter(source(i).read_text())
        if any(re.search(rf"^{k}:", fm, re.M) for k in ROOT_KEYS):
            closures.append(i)

    results = []

    def record(kind, name, ref, got, diffs):
        # Order is a documented change, not a difference: required reads
        # are listed in declaration order and a closure depth-first in
        # declaration order (progressive-retrieval § Get, § Closure), where
        # the predecessor sorted them by taxonomy.
        changes = [d for d in diffs if d.get("class") == "order" and kind != "index"]
        diffs = [d for d in diffs if d not in changes]
        results.append({"kind": kind, "name": name, "differences": diffs, "documented_changes": changes})
        if diffs or changes:
            safe = re.sub(r"[^A-Za-z0-9._-]+", "__", f"{kind}--{name}")
            (out / "diffs").mkdir(exist_ok=True)
            (out / "diffs" / f"{safe}.json").write_text(
                json.dumps({"differences": diffs, "reference": ref, "instrukt": got}, indent=2)
            )

    head_sources: dict[str, str] = {}

    def stale(bid, ref_only, ins_only):
        """Whether the predecessor's reference is older than its committed
        HEAD: what only instrukt shows is in the HEAD source, and some of
        what only the reference shows is not. The reference serves its live
        home, which may have been published from an earlier commit."""
        if bid not in head_sources:
            rel = (
                "docs/local/" + bid.removeprefix("local/projects/teleclaude/")
                if bid.startswith("local/projects/teleclaude/")
                else "docs/global/" + bid
            ) + ".md"
            code, text, _ = run(["git", "show", f"HEAD:{rel}"], pred)
            head_sources[bid] = norm(text) if code == 0 else ""
        src = head_sources[bid]
        if not src:
            return False
        ins = [l.strip() for l in ins_only if l.strip()]
        ref = [l.strip() for l in ref_only if l.strip()]
        # Only instrukt's lines are all in HEAD, and either the reference
        # lacks HEAD lines outright or shows lines HEAD no longer has.
        return bool(ins or ref) and all(l in src for l in ins) and (not ref or any(l not in src for l in ref))

    def head_slugs(bid):
        """The todos with pending material for `bid` at the predecessor's
        HEAD: its planned markers and its companions."""
        stale(bid, [], [])
        slugs = set(re.findall(r"<!--\s*planned(?:-change)?:([a-z0-9-]+)\s*-->", head_sources[bid]))
        p = source(bid)
        slugs |= {c.name[len(p.stem) + 1 : -3] for c in p.parent.glob(f"{p.stem}.*.md")}
        return slugs

    def body_class(bid, ref_only, ins_only):
        """A first reading of why two bodies differ; `None` when no rule
        explains it. The report confirms each."""
        if bid.endswith("/bundle"):
            # Compiled by publication on each side, from each side's commit.
            return "bundle-compilation"
        if ref_only and all("@<REPO>/" in l for l in ref_only) and not any("@<REPO>/" in l for l in ins_only):
            return "reference-in-code-kept"
        if ref_only and all("<!-- @include:" in l for l in ref_only if l.strip()):
            return "include-expanded"
        if stale(bid, ref_only, ins_only):
            return "reference-stale"
        return None

    def reads_class(ref, got):
        """Why two lists of required reads or blocks differ, when a rule
        explains it."""
        ref_only = [i for i in ref if i not in got]
        got_only = [i for i in got if i not in ref]
        renamed = {f"<HOME_DOCS>/{i.removesuffix('/bundle')}/bundle.md" for i in ref_only if i.endswith("/bundle")}
        if renamed and renamed == {i for i in got_only if i.endswith("/bundle.md")}:
            got_only = [i for i in got_only if i not in renamed]
            ref_only = [i for i in ref_only if not i.endswith("/bundle")]
            if not got_only and not ref_only:
                return "bundle-named-by-path"
        if not ref_only and any(i.startswith("local/") or i.endswith("/bundle.md") for i in got_only):
            return "reads-the-predecessor-omits"
        return None

    def compare_get(kind, name, args):
        rc, ro, re_ = telec(args)
        ic, io_, ie = instrukt(args)
        ro, re_, io_, ie = map(norm, (ro, re_, io_, ie))
        diffs = []
        if (rc == 0) != (ic == 0):
            diffs.append({"what": "exit", "reference": rc, "instrukt": ic, "stderr": [re_, ie]})
        r, g = parse_get(ro), parse_get(io_)
        for key in ("not_loaded", "included"):
            a, b = r["reads"][key], g["reads"][key]
            if set(a) != set(b):
                withheld = any(any("withheld" in l for l in blk["notice"]) for blk in g["blocks"])
                diffs.append(
                    {
                        "what": f"required reads ({key})",
                        "reference": a,
                        "instrukt": b,
                        "class": "withheld-notice" if withheld and not b else reads_class(a, b),
                    }
                )
            elif a != b:
                # progressive-retrieval names no order for the list.
                diffs.append(
                    {"what": f"required reads order ({key})", "reference": a, "instrukt": b, "class": "order"}
                )
        # A bundle block is keyed by domain on both sides: the predecessor
        # names it `<domain>/bundle`, instrukt `bundle: <domain>`.
        def key(b):
            f = b["fields"]
            return f.get("id") or (f"{f['bundle']}/bundle" if "bundle" in f else None)

        rb = {key(b): b for b in r["blocks"]}
        gb = {key(b): b for b in g["blocks"]}
        if set(rb) != set(gb):
            only_g = [k for k in gb if k not in rb]
            withheld = all(any("withheld" in l for l in gb[k]["notice"]) for k in only_g)
            diffs.append(
                {
                    "what": "blocks",
                    "reference": list(rb),
                    "instrukt": list(gb),
                    "class": "withheld-notice"
                    if only_g and withheld and not [k for k in rb if k not in gb]
                    else reads_class(list(rb), list(gb)),
                }
            )
        elif list(rb) != list(gb):
            # progressive-retrieval names no order for a reading's blocks.
            diffs.append({"what": "blocks order", "reference": list(rb), "instrukt": list(gb), "class": "order"})
        for bid in rb.keys() & gb.keys():
            a, b = rb[bid], gb[bid]
            bundle = "bundle" in b["fields"]
            for f in sorted(set(a["fields"]) | set(b["fields"])):
                # A bundle block's header names it differently on each side
                # (reported once as the bundle header shape).
                if f in ("bundle", "path") or (bundle and f in ("id", "type")):
                    continue
                if a["fields"].get(f) != b["fields"].get(f):
                    is_stale = stale(bid, [a["fields"].get(f) or ""], [b["fields"].get(f) or ""])
                    diffs.append(
                        {
                            "what": f"{bid} field {f}",
                            "reference": a["fields"].get(f),
                            "instrukt": b["fields"].get(f),
                            "class": "reference-stale"
                            if is_stale
                            else "third-party-scope-omitted"
                            if bid.startswith("third-party/") and f == "scope" and b["fields"].get(f) is None
                            else None,
                        }
                    )
            if a["body"] != b["body"]:
                ra, ga = a["body"].splitlines(), b["body"].splitlines()
                ref_only = [l for l in ra if l not in ga]
                ins_only = [l for l in ga if l not in ra]
                diffs.append(
                    {
                        "what": f"{bid} body",
                        "reference": a["body"],
                        "instrukt": b["body"],
                        "class": body_class(bid, ref_only, ins_only),
                    }
                )
            if bool(a["notice"]) != bool(b["notice"]):
                diffs.append(
                    {
                        "what": f"{bid} notice",
                        "reference": a["notice"],
                        "instrukt": b["notice"],
                        "head_pending_slugs": sorted(head_slugs(bid)),
                        "class": "reference-stale"
                        if bool(b["notice"]) == bool(head_slugs(bid))
                        else None,
                    }
                )
        record(kind, name, {"exit": rc, "stdout": ro, "stderr": re_}, {"exit": ic, "stdout": io_, "stderr": ie}, diffs)

    for i in ids:
        compare_get("get", i, ["docs", "get", i])
    for i in closures:
        compare_get("closure", i, ["docs", "get", i, "--closure"])

    # Ids the domains' progressive baselines name (docs domain spec, Baseline
    # manifest: `baseline.md` and `baseline-progressive.md`).
    progressive = set()
    for manifest in (clone / "docs" / "global").glob("*/baseline-progressive.md"):
        for line in manifest.read_text().splitlines():
            m = re.match(r"^@~/\.instrukt/docs/(.+)\.md$", line.strip())
            if m:
                progressive.add(m.group(1))

    def head_entry(k):
        """The index entry line the predecessor's HEAD source implies: its
        description and its pending suffixes."""
        if k.startswith("third-party/"):
            return None
        stale(k, [], [])
        fm = frontmatter(head_sources[k])
        m = re.search(r"^description: ?(.*)$", fm, re.M)
        desc = m.group(1).strip().strip("'\"") if m else ""
        line = re.sub(r"\s+", " ", desc).strip()
        d = re.search(r"^delivered_by: ?\[(.*)\]$", fm, re.M)
        if d:
            line += f" (delivered by: {', '.join(s.strip() for s in d.group(1).split(','))})"
        slugs = sorted(head_slugs(k))
        if slugs:
            line += f" (superseded by: {', '.join(slugs)})"
        return line

    def suffixes(line):
        sup = re.search(r" \(superseded by: ([^)]*)\)", line)
        text = re.sub(r" \((?:delivered|superseded) by: [^)]*\)", "", line)
        return text, sorted(x.strip() for x in sup.group(1).split(",")) if sup else []

    def entry_stale(k, ref, got):
        """Whether the reference entry is older than the predecessor's
        HEAD: instrukt's entry is what HEAD implies, or its description
        matches and only its superseded-by slugs differ, as HEAD has them."""
        if got == head_entry(k):
            return True
        (rt, rs), (gt, gs) = suffixes(ref), suffixes(got)
        return rt == gt and gs == sorted(head_slugs(k)) and rs != gs

    def compare_index(name, args, scope):
        _, ro, _ = telec(["docs", "index", *args])
        _, io_, _ = instrukt(["docs", "index", *args])
        ro, io_ = norm(ro), norm(io_)
        r = {k: v for k, v in entry_lines(ro).items() if scope(k)}
        g = {k: v for k, v in entry_lines(io_).items() if scope(k)}
        diffs = []
        for k in sorted(r.keys() - g.keys()):
            diffs.append({"what": "missing in instrukt", "id": k, "reference": r[k]})
        for k in sorted(g.keys() - r.keys()):
            diffs.append(
                {
                    "what": "extra in instrukt",
                    "id": k,
                    "instrukt": g[k],
                    "class": "progressive-baseline" if args == ["--baseline-only"] and k in progressive else None,
                }
            )
        for k in sorted(r.keys() & g.keys()):
            if r[k] != g[k]:
                diffs.append(
                    {
                        "what": "entry",
                        "id": k,
                        "reference": r[k],
                        "instrukt": g[k],
                        "class": "reference-stale" if entry_stale(k, r[k], g[k]) else None,
                    }
                )
        ro_order = [k for k in entry_lines(ro) if scope(k) and k in g]
        go_order = [k for k in entry_lines(io_) if scope(k) and k in r]
        if ro_order != go_order:
            diffs.append({"what": "order", "reference": ro_order, "instrukt": go_order})
        record(
            "index",
            name,
            {"stdout": ro, "header": header_lines(ro)},
            {"stdout": io_, "header": header_lines(io_)},
            diffs,
        )
        return header_lines(ro), header_lines(io_)

    headers = compare_index("unfiltered", [], ours)
    for d in names:
        compare_index(f"--domains {d}", ["--domains", d], ours)
    for t in TAXONOMIES:
        compare_index(f"--areas {t}", ["--areas", t], ours)
    compare_index("--baseline-only", ["--baseline-only"], ours)
    compare_index("--third-party", ["--third-party"], ours)

    passed = sum(1 for r in results if not r["differences"])
    summary = {
        "predecessor_head": pred_head.strip(),
        "domains": names,
        "ids": len(ids),
        "closures": len(closures),
        "index_views": sum(1 for r in results if r["kind"] == "index"),
        "compared": len(results),
        "passed": passed,
        "differing": len(results) - passed,
        "normalisations": NORMALISATIONS,
        "index_header": {"reference": headers[0], "instrukt": headers[1]},
        "results": results,
    }
    (out / "summary.json").write_text(json.dumps(summary, indent=2))
    lines = [
        f"# Core parity: predecessor {pred_head.strip()[:12]}",
        "",
        f"- ids: {len(ids)}; closures: {len(closures)}; index views: {summary['index_views']}",
        f"- compared: {len(results)}; passed: {passed}; differing: {len(results) - passed}",
        "",
        "## Differences by kind",
        "",
    ]
    kinds: dict[str, int] = {}
    for r in results:
        for d in r["differences"]:
            what = re.sub(r"^\S+/\S+ ", "", d["what"])
            what += f" [{d.get('class') or 'unclassified'}]"
            label = f"{r['kind']}: {what}"
            kinds[label] = kinds.get(label, 0) + 1
    lines += [f"- {k}: {v}" for k, v in sorted(kinds.items())] + [""]
    summary["by_kind"] = kinds
    changed: dict[str, int] = {}
    for r in results:
        for d in r["documented_changes"]:
            label = f"{r['kind']}: {re.sub(r'^\S+/\S+ ', '', d['what'])}"
            changed[label] = changed.get(label, 0) + 1
    lines += ["## Documented changes (not differences)", ""]
    lines += [f"- {k}: {v}" for k, v in sorted(changed.items())] + [""]
    summary["documented_changes"] = changed
    (out / "summary.json").write_text(json.dumps(summary, indent=2))
    for r in results:
        if r["differences"]:
            lines.append(f"## {r['kind']} {r['name']}")
            for d in r["differences"]:
                lines.append(f"- {d['what']}" + (f" `{d['id']}`" if "id" in d else ""))
            lines.append("")
    (out / "summary.md").write_text("\n".join(lines))
    print(f"compared {len(results)}: {passed} passed, {len(results) - passed} differing; {out}")
    return 0


if __name__ == "__main__":
    sys.exit(main())
