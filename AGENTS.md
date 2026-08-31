# The working system

A reusable way of running agent work on a repo. Nothing here is specific to one
project. A project adopts it with `adopt.sh` (idempotent: symlinks, doc-web
skeleton, gitignore entries) or by hand — the role definitions go in the
directory the coding harness in use reads them from (Claude Code:
`.claude/agents`; pi.dev: `.pi/agents`):

    # either: ./adopt.sh <project>

This will create the necessary starter docs, copy over AGENTS.md and symlink the agents.

Every rule below was paid for. Where a rule carries a date or a quote, that is
the scar it came from; keep it, because a rule with a scar gets followed.

---

## 1. Roles

Roles are defined by **what they may write** and **whether they may move trunk**.
That is the whole model. Full definitions are in `agents/`.

| Role | Works in | Writes | Moves trunk? |
|---|---|---|---|
| `coder` | own workspace | production code, its tests, the rows recording it | no |
| `researcher` | own workspace | `docs/research/` only; no production code | no |
| `tech-lead` | own workspace | `docs/design/` only; never lands without human review | no |
| `designer` | own workspace | UI, plus rendered before/after evidence | no |
| `foreman` | own workspace, dispatches lanes | briefs and track state; little code | no |
| `scribe` | the default workspace | `BOARD.md`, and rows in the other ops docs | no |
| `integrator` | the default workspace | conflict resolutions only | **yes — sole authority** |
| `patchbay` | the conversation | the board, the briefs | via `integrator` |

**Why per-role and not per-task.** An agent roster is read once at session start,
so a name invented for one task is always a session late. There are eight roles,
they are permanent, and they are always available.

**The human is not one of the six.** He sets direction and priorities, reviews,
and **edits the board and the ideas inbox freely at any time** — so an agent
re-reads those documents before acting on them rather than trusting a copy it
read earlier. He launches the application himself; agents never do (it steals his
screen focus).

Two rules hold the model together.

**A lane rebases itself onto trunk and stops.** Its last act is to rebase, re-run
its gates *after* the rebase, and report. It does not land itself. One writer to
the bookmark — otherwise you get N-way races on trunk, which is how a repo ends
up with conflicted commits inside its own history.

**Trunk holds still while lanes are rebasing onto it.** Moving it underneath a
running lane corrupts nothing, but it silently invalidates that lane's gate run,
and green on a base nobody will integrate is not evidence. On 2026-08-29 one lane
rebased three times and re-resolved the same conflict twice because this was
ignored. Batch integrator commits; land them after lanes report.

**Anything over ~15 minutes of engineering leaves the conversation.** The
coordinator that talks to the human stays free; long serial work done inline is
dead time for him (*"you're doing too much stuff in main agent, I am blocked by
you linearizing all this work"*). Spawn first, then do the small residue.

---

## 2. The doc web

A project run this way keeps a small set of documents. Most of these rules exist
because something ended up in the wrong one, so each says what does **not**
belong as well as what does.

**These documents carry no policy of their own.** What belongs in each, and what
does not, is stated once — here — and a document that restates it drifts from
this list and pollutes the content it exists to hold (Chris, 2026-08-29: *"the
operational sections at the top of what belongs in which doc … belong in
AGENTS.md and pollute those docs"*). Each opens with one pointer line and, where
its rows carry status marks, the legend for reading them: a key belongs beside
its table, a policy does not.

### `BOARD.md` — the live board

The concept matters more than the file. It is a **shared, live document that
every agent reads before acting and writes when its state changes**, and it is
the human's primary view of what is happening and what needs them.

- **Line-items only.** Prose is what lanes collide on: when eight lanes each
  edited the same narrative paragraph, every one of them conflicted. Removing the
  prose took one board from 404 lines to 233 and the collisions stopped.
- **In-flight state does not live here at all** — it lives in the live file
  above, which is why the `integrator` no longer owns a section of this
  document and the `scribe` no longer waits for one. A lane
  never touches another lane's rows anywhere on the board.
- **Keep it short.** It exists to show the *live* work, so a landed lane's row
  moves out of the queue and into the landed index the same day.
- Sections: in flight · milestones · queued · deferred by decision · landed index.
- **Not here**: rationale, discussion, anything addressed to the human. Those go
  to the human channel below. A board entry is a row, a state and a label.
- **Whatever is waiting on the human lives here** — not in a commit message and
  not in a banner on a file. This is the document he reads for exactly that, and
  it can be edited when the answer arrives.

**Resolve board conflicts by hand, not by script** — read the file and write the
intended result, then re-read the section you touched. It is one file, prose plus
tables, and the file everyone navigates by; script-merging it saves a minute and
costs the thing its value. The scars are under *Landing*, rule 3.

**A label is something the human types.** Never strike a label through — a
`~~LABEL~~` cannot be typed to fire a lane, tildes are awkward to reach, and a
struck row reads as *present but crossed out* when what is meant is *gone*. A
landed lane's row is **deleted** from Queued and its name appears in the landed
index; a superseded one is deleted and its successor says what it replaced.
Status lives in the status column and in which section the row is in, never in
the label (Chris, 2026-08-29: *"please don't use tildes in names, not sure why
you're doing that, tildes are hard to type"*).

### The live directory — untracked, and the churny half of the board

The board splits in two, because its two halves have opposite needs.

**`BOARD.md` is tracked and slow**: milestones, queued, deferred, the landed
index. Every lane can read it in its own workspace and write its own row, and
conflicts are rare once the prose is gone.

**The scratch directory is gitignored and fast** (`docs/scratch/`, holding `STATUS.md`): what is running, what is ready and
unintegrated, where trunk is, what is waiting on the human. It changes hourly.

Prefer a gitignored **directory** over a gitignored file: one ignore entry, and
room for whatever else turns out to want the same treatment.

The reason for the split is not tidiness. Both a `scribe` and an `integrator`
write in the default workspace and must not run at once, so while a long
integration runs, board updates queue behind it — and that is exactly when most
is changing and the human most wants to look. A **gitignored** file does not
dirty the working copy at all, so the `scribe` can rewrite it *during* an
integration. Splitting removes the contention rather than scheduling around it.

The trade, stated plainly: **a gitignored file is invisible to lanes**, because
each lane works in its own workspace directory. That is acceptable only because
a lane gets its context from its brief, not from the board. Anything a lane
must be able to read stays in the tracked half.

**The tracked half must point at it.** A gitignored directory nobody references
is undiscoverable — the board and the project's `AGENTS.md` both name it and say
what lives there, even though neither can see its contents.

### `CHANGELOG.md`

What got added, per landing, dated and tied to versions — *"what got added, not
all the analysis and story"* (Chris, 2026-08-29). What a capability now is, what
changed in behaviour — especially anything that alters saved files or an API —
and what was fixed, in a line. A few lines an entry. In a merge, **both** entries
survive, the later landing above the earlier.

The displaced material has homes and should reach them rather than be cut:
measurements, root causes and alternatives weighed → `docs/research/`; what a
lane left undone → a board queue row or an issues row; a decision that was *made*
rather than applied → `DECISIONS.md`, linked from the entry. **An entry that
reads as a narrative is in the wrong document.**

**Not here**: work that has not landed; roadmap intent.

### Dating an entry — clock *and* change id

A date alone is not enough granularity. Thirteen lanes landed on one date in this
project's history, and two changelog entries had to be reordered by hand because
nothing in the text said which came first.

Every `CHANGELOG.md` entry and every `DECISIONS.md` row carries **both**:

    #### A preset that describes itself (`LIBFIN`) · 2026-08-29 20:15 · oplmqxmn
    | 2026-08-29 20:15 · uvzmuvkl | **The analysis toolkit is a separate crate family…** |

- **The clock** is for a human skimming: roughly when this happened. It is
  written by the lane at commit time, because the lane is the only party that
  knows, and it is *not* the ordering authority.
- **The change id** is the ordering authority and the verification. Ancestry
  answers "which landed first" exactly — with no ambiguity about *which* moment
  a timestamp recorded, since authoring, rebasing and landing are often hours
  apart. And `jj show <id>` produces the actual change, so a prose record can be
  checked against the tree rather than drifting from it.

**This works because jj change ids survive rebase.** A lane knows its id when it
writes the entry, and the id is still valid after five rebases onto a moving
trunk. A commit hash would not be; do not use one.

### `ISSUES.md`

Defects and rough edges anyone notices in passing: file it, triage it, **DON'T
interrupt a lane for it**. A **parking lot, not a queue** — nothing here is
scheduled until it is promoted onto the board. Filed by whoever found them,
closed by whoever fixed them, and **never silently closed**. A resolved issue
loses its long description, keeps a one-line verdict, and points at the changelog
for the account, so the table reads as what is still open.

**Not here**: features somebody would ask for by name (→ ideas), or work already
scheduled (→ board).

### `IDEAS.md`

An inbox, in three sections: **Inbox → In-flight → Done**. Ideas arrive raw and
are allowed to be half-formed; that is the point of an inbox. Append a row the
moment an idea surfaces; promote it when it earns a lane. **An idea only ever
moves down** — Inbox (no commitment) → In-flight (it has a board row) → Done
(answered; the account is in the changelog). Ids are stable and never reused, so
a row keeps its number as it moves.

### `ROADMAP.md`

Horizons and versions, and what a version *means* in the register of the ones
before it — built from the ideas story lines, plus the dependencies that gate
them. Horizon numbers continue the version series. **Only what is not done.**

**Not here**: finished work — that is the changelog, version by version.

### `DECISIONS.md`

Dated decisions with their rationale, so a settled question stays settled and
nobody relitigates it three sessions later. One row, one decision, one date.

### `docs/design/`

Proposals and the locked design. **Gated on human review: a design document never
lands on trunk without it.** While a proposal sits unlanded in a lane's own
chain, it carries a banner saying it awaits review; once it is on trunk, its
state lives on the board instead, because a banner claiming a proposal is
unreviewed outlives the review.

**Changing a design needs review; recording that it was built does not.** Once a
design is approved, a lane marking its own stage as built, correcting a sketch to
match the code, or adding a finding the design did not predict is **reporting,
not proposing**, and does not wait on a second review. Gating those turned an
approved document into one that read as pending for hours after it had been
signed off, in the document the human would look at to check (Chris, 2026-08-29:
*"why is SEQUENCER.md still waiting"*). A lane that wants to *change* what was
decided raises it on the board, not as a banner on the file.

If a design's section numbers are cited from source code, **they are never
renumbered.**

### `docs/research/`

Investigations and measurements. A number with its method beside it settles an
argument; a paragraph of plausibility does not. Also **the argument behind
anything designed but not built**.

**There is no register of unlanded designs** (dissolved 2026-08-29, Chris: *"I
kind of feel like DESIGN.md as a doc should not exist unless there are things
that don't fit into the docs I just mentioned"* — nothing in it did). An unlanded
design is **a research note plus a queue row**: the note holds the argument and
the measurements, the row holds what a lane would do and what it costs, and the
decision it waits on lives where it can still be answered — a board row's
priority field, an ideas note, or a decisions row once it is answered. When it
lands it moves into the design document.

### `docs/guides/` · `docs/reference/` · `docs/archive/`

How to do a thing — build, run, release, use a tool. Generated references say so
and are **regenerated, not edited**. Historical material is archived rather than
deleted.


### The human channel — gitignored

A directory (for example `docs/scratch/`) for notes between the human and
the agents. **Gitignored, so that conversation never enters history.** This is
what lets the board be line-items at all: the narrative has somewhere to go.

It also holds **commands only the human can run**, with the context to judge each
one. `rm` is denied to agents, so deletions land here rather than in a chat
message (Chris, 2026-08-29: *"copy paste from Claude Code is terrible"*). **A
block that names paths names them explicitly** — an *"everything except…"* filter
is evaluated when the human runs it, in a world the agent could not see, which is
how a cleanup deletes a lane started after it was written.


### Where they live

Reshaped 2026-08-29 (Chris: *"where should the remaining top-level docs go?
Ideally only one or 2 key ones remain"*):

- **the repo root** holds `AGENTS.md` and nothing else of this kind;
- **one directory** holds everything that *decays* — board, changelog, issues,
  ideas, roadmap, decisions. These are the documents that are wrong the moment
  work lands, which is why they live together and get maintained together;
- everything else lives by kind: design, research, guides, reference, archive.

Documentation lives in the docs tree, **never inside the source tree**.

---

## 3. Default directory structure

    AGENTS.md                  points at this file, then this project's own facts
    .claude/agents ->          symlink to ~/projects/simply/dev/agents
                               (Claude Code; pi.dev uses .pi/agents instead)
    docs/
      ops/                     BOARD.md CHANGELOG.md ISSUES.md IDEAS.md
                               ROADMAP.md DECISIONS.md
      design/                  proposals — human review gate
      research/                investigations and measurements
      guides/                  how-to
      reference/               generated — regenerate, don't edit
      archive/                 historical
      scratch/                 gitignored notes and human-only commands
    .workspaces/<lane>/        one per lane, gitignored, INSIDE the repo
    .screenshots/              gitignored
      progress/<date>-<LANE>/  what a lane renders; only scenes that CHANGED
      baseline/<date>/         the full set after an integration; keeps everything

Workspaces go **inside** the repo, not `../` siblings — siblings pollute the
directory the other repos live in.

---

## 4. Operating rules

### Before you land anything: check trunk is where you think it is

**Verify the bookmark against the actual tip.** After a history operation — an
`op restore`, an undo, a rebase someone else ran — a bookmark can be left
pointing at an old commit while the real work sits above it. One was found **152
commits behind** the tip that `git log` agreed with. Nothing warns you; every
rebase you then run is onto the wrong base. Check first, every session.

**Know what is actually unintegrated, from a query, not from memory.** Ask the
repo — in `jj`, `ancestors(<workspace>@) ~ ::<trunk> ~ empty()` — and count. Two
lanes were recorded as landed on a board when neither had been. A workspace whose
working copy is *empty* is normal and means nothing: a finished lane's work sits
in the ancestors, not in `@`.

**Bookmark a chain's tip before forgetting its workspace.** Forgetting the
workspace removes the only thing keeping that chain visible.

**One workspace, one writer — the default workspace included.** While an
`integrator` is landing, nobody else writes in the default workspace, and that
includes whoever is dispatching. This is the sibling of *trunk holds still while
lanes are rebasing onto it*, and it was broken within the hour of that one being
written: on 2026-08-29 the dispatcher created a symlink and edited a tracked
ignore file in the default working copy mid-integration. The integrator found
unexplained changes in its own working copy, could not tell whose they were, and
had to park them as a side commit to keep `@` clean. Nothing was lost, but it
spent its judgement on an archaeology problem that should not have existed. If
you need a change made while an integrator holds the workspace, hand it to the
integrator or wait.

### Trunk and the working copy

**`default@` is always an empty commit one above trunk.** The integrator's
working copy is scratch, never the thing that ships. The loop is Chris's
(2026-08-29):

    jj new                      # @ is an empty child of main
    …edit…
    jj commit -m "…"            # describes @ AND opens a fresh empty child
    jj bookmark set main -r @-  # main is the commit just made, @ stays empty

`jj commit` is the point: it does the describe and the `jj new` in one step, so
there is no window in which `@` is a described commit waiting to be pointed at.
`@-` then names it, and `@` is empty again without a second command. Two things
go wrong when the loop is broken, and both are invisible until someone reads the
log:

- an edit made while `@` *is* trunk silently joins the last commit — which is how
  two unrelated changes ended up sharing a message three times in one day;
- a `jj describe` on a commit the bookmark already points at **overwrites** its
  message rather than starting a new one — which is how a landed lane's account
  was replaced by the next thing typed.

**The integrator does not run into a version.** When the last lane of a milestone
lands, stop and hand back rather than continuing to the tag (Chris, 2026-08-29:
*"don't go there automatically, let's take the opportunity for a human review and
cleaning up old workspaces"*). Every lane was reviewed against its own brief and
nobody has read the result as one thing; a version boundary is the natural place
for that and the last cheap one, because after a tag a correction is a change to
something released. It is also the only moment when no lane is rebasing, which is
what workspace cleanup and any history repair need.

### The lane lifecycle

1. Idea or direction lands → **docs first** (an ideas row, a board queue entry, a
   decisions entry as appropriate) before engineering starts.
2. Launch: `jj workspace add .workspaces/<lane> --name <lane>`, plus a brief.
3. The lane works: incremental commits on its own chain, with the project's
   version control and **never git in a colocated repo** — a `git commit` after
   `jj new` lands on a detached head. Chains are crash-safe: they survive a dead
   agent, and a replacement resumes from the same fence.
4. **Every finishing lane records its own landing on the board as part of its
   final commit** (Chris, 2026-08-28): its queue row updated, and any follow-up
   work it discovered added to the queue with a priority. The integrator then
   only resolves board conflicts instead of reconstructing every lane's state by
   hand — which does not scale past a handful of lanes and is where tracking
   actually broke down. The "in flight" section stays the integrator's.
5. **The lane's last act is to rebase itself onto trunk**, resolve every conflict,
   and **re-run the gates afterwards**. Then report, saying what conflicted and
   how it was resolved. Then stop.
6. A `integrator` lands it: re-read the board for the human's edits → rebase the
   chain onto trunk → resolve → gates unfiltered → advance the bookmark → update
   the board and append the changelog entry → tell the human.
7. Recycle: forget the workspace when a lane is done. Directory deletion is the
   human's (agents cannot `rm`); each stale workspace holds a multi-GB build
   cache, so forget and delete promptly.

**Why the rebase belongs to the lane and not the integrator** (Chris, 2026-08-29):

- **The lane knows what its own change meant.** The integrator resolving someone
  else's conflict is guessing at intent, and a wrong guess in a union merge is
  silent — it produces a plausible document nobody wrote. That has happened more
  than once: a merged board entry lost its opening line, an entry's body was
  reattached to the wrong heading, and a changelog account ended up describing
  another lane's work.
- **A rebase can break a lane that merged cleanly.** Another lane may have
  changed the API this one calls, so a conflict-free rebase is not a green one.
- **Resolving at the tip is not resolving.** See below.

The integrator's job is then what it should be: read the report, move the
bookmark, verify.

**Never rebase a RUNNING agent's chain** (learned 2026-08-28). Integrating a
lane's chain while that agent is still working corrupts its working copy: the
stale-workspace recovery fires under it and can revert it to an older revision,
leaving its change divergent. One survey lost work this way and only recovered
from scratchpad backups. **Integrate only chains whose agent has reported and
stopped.** If a lane must land early, message it to commit and stop FIRST, wait
for the completion notification, then rebase. The integrator's own working copy
going stale is harmless by comparison — that is just this session catching up.

### Landing

1. **Resolve the oldest commit in a chain first, never the tip.** A clean tip can
   sit on conflicted ancestors. Resolving at the tip is what put nine conflicted
   commits into one repo's own history.
2. **Read the label on every conflict block; never assume which side is which.**
   `jj` writes `+++++++ … (rebase destination)` for trunk's side but
   `(rebased revision)` for the lane's — and it alternates *within a single
   file*, sometimes between two blocks of the same document. A repo has already
   been corrupted once by assuming, and the union-merge that did it looked right.
3. **Never resolve a shared document with a union script.** A blind union (keep
   `+` lines, drop `-` lines, keep context) **silently mangles entries**: a row
   both sides edited loses the unchanged half, and a `-`/`+` rewrite of the same
   row can leave the old row duplicated elsewhere. It has duplicated a heading,
   orphaned one lane's body under another lane's heading, and dropped a
   correction entirely. Two such defects reached trunk on 2026-08-28 — a merged
   entry stripped of its opening line, and a superseded queue row appearing twice
   — and were only caught because a merge agent read the file properly. Worse,
   the damage survives: a board was found carrying **two `## Queued` tables with
   different column headers**, from a union merge sessions earlier that nobody
   noticed. After resolving any document by machine, **grep for duplicated
   headings.**
4. **A clean textual merge can still leave a semantic conflict** — a changed
   signature, a moved function, an enum that gained an arm. Two real ones from a
   single integration: a trait impl added on trunk kept the old method signature
   after a clean merge and the crate would not compile; and a lane passed a value
   to a function whose parameter type another lane had turned into an enum. The
   gates are the only proof, and neither lane could have seen its own.
5. **Deletion-versus-modification is the reorg conflict.** When trunk splits a
   file into a directory and a lane edited the old file, the conflict presents as
   the entire file recreated with markers — a two-line change can look like a
   thousand-line conflict. **Extract the lane's real `+`/`-` delta first**, take
   trunk's deletion, then relocate that delta into whichever new module owns the
   responsibility, and say where you put it. Where `rm` is denied, accept the
   deletion with a restore-from-trunk of that path.
6. **Import lists are the most common code conflict** after a reorg. The merge is
   almost always *trunk's form, minus what the lane removed, plus what the lane
   added* — then verify the removed symbol is genuinely unused rather than
   trusting the lane.
7. **When trunk changes a module's visibility, decide by the rule, not by
   copying a side.** Is the module reached by path from another crate? Are its
   contents re-exported at the root? Answer those and the visibility follows;
   picking whichever side looks tidier reintroduces exactly what a
   surface-cleanup lane just removed.
8. **`jj squash` moves the whole working copy, not the file you had in mind.**
   Pass explicit paths when the working copy holds more than one concern —
   otherwise an unrelated asset lands inside a documentation commit, and you will
   not notice until the commit after it reports `(empty)`.
9. **`default@` is always an empty commit one above trunk.** After landing, make
   a new empty commit — see *Trunk and the working copy* above for the loop and
   the two ways breaking it corrupts a message.
10. **Never abandon a commit trunk points at without re-setting the bookmark.**
    Abandoning deletes it, and the next command fails with "revision doesn't
    exist" rather than anything that explains itself.

### Ordering and document conventions

- `CHANGELOG.md`: keep **both** entries; the **later-landing** lane's goes above.
  Getting this backwards is easy when both are dated the same day — order by when
  each landed, not by which side of the conflict it came from.
- `ISSUES.md` / `DECISIONS.md`: keep both sides' rows. Where a lane rewrites one
  row (a defect it fixed), match that row by its opening text and swap it —
  never by line position, which trunk will have moved.
- `BOARD.md`: line-items only, and its "in flight" section is the `integrator`'s to
  rewrite wholesale. Lanes take trunk's side there.
- **Never land a `docs/design/` change** — those are the human's to review.

### Repairing history, if it comes to that

Conflicts stored in a commit's own tree are history quality, not a broken tree:
the tip can be clean and everything can build while ancestors carry markers. Fix
it only under three conditions, and the first two are why it usually waits.

- **No lane may be running.** Every lane rebases onto trunk, so rewriting it
  underneath them invalidates their work.
- **Resolve oldest-first, in a scratch workspace.** Three attempts in one session
  turned one conflicted commit into 11, then 71; both were undone with an
  operation restore, which is the reason to work where an undo is cheap.
- **The acceptance test is the tree, not the log.** Hash the documents and the
  source before and after; the result must be **byte-identical**. A repair that
  changes content is a different change wearing a repair's clothes.

### Evidence

- **Run the project's gates unfiltered and report them verbatim**, counts
  included. A filtered gate is not a gate.
- **Re-run gates after the final rebase**, not before.
- Never report green without having run them.
- **Verify headlessly.** Never auto-launch the application to check something —
  it steals the human's screen focus. He launches it himself.

### What a commit message says

**A commit message says what the commit did — never what it is waiting for.**
Review state, "PROPOSAL", "NEEDS REVIEW" and the like do not belong in a commit,
least of all in its subject: a commit is immutable and review state changes, so
the message is wrong within hours and stays wrong forever (Chris, 2026-08-29:
*"calling out for 'needs review' in commit messages is silly, that's what
BOARD.md is for"*). No co-author lines.

### Survival

- **Commit early and often.** A WIP commit in your own workspace costs nothing.
  Four agents died mid-task on a spend limit on 2026-08-29 and lost everything
  between them because none had committed. A crash should cost minutes.
- **Reuse an existing workspace rather than creating one.** Creating and deleting
  workspaces is expensive.

### Environment

- **Agents must run version-control commands unsandboxed.** A sandbox that
  intercepts filesystem syscalls turns a working-copy scan into minutes: `jj log`
  measured **over two minutes** sandboxed and **16 ms** without. It hangs rather
  than fails, so it reads as a repo problem — the same command run by hand
  returns instantly, which is how it was finally spotted. A slow `jj` is not
  evidence of a large repo, a lock, or a corrupt operation log. Check this first.
- **`jj squash` and `jj describe` open `$EDITOR` and hang an agent forever.**
  `export JJ_EDITOR=true` and always pass `--use-destination-message`.
  `--use-destination-message` is the real fix; `JJ_EDITOR` is the belt.
- **`rm` may be denied.** Write needed deletions into the human channel.

### Work

- **Linear history on trunk** — rebase chains, never merge commits.
- **DRY.** Writing the same thing twice is the signal to move it one level down.
- **Simplify by generalizing.** Prefer the change that makes the system smaller
  AND more general: unify parallel mechanisms behind one clean abstraction rather
  than adding a sibling mechanism. Every proposed abstraction carries a
  *simplification ledger* — concepts and lines removed versus added — and is
  rejected if it does not come out ahead. Keep aspects orthogonal and
  decouplable: pure-data contracts, separable traits, operations and events as
  serializable values instead of callbacks, so pieces compose without knowing
  each other.
- **Parallelize; migrate later.** Refactors do not gate feature lanes; whichever
  lands second owns migrating its slice. Accepted trade-off: merge conflicts over
  idle lanes. A refactor lane keeps risky rewires **additive-first,
  cutover-last** so parallel chains merge textually.
- **Docs before eng; externalize by default.** Decisions, ideas, plans and
  principles never live only in a context window; capture them per the doc web
  the moment they occur.
- **Don't widen your own scope.** If work turns out much larger than briefed,
  land what is coherent and say plainly what you left and why. Scaling the job
  down is the human's call.
- **Markdown tables**: fixed-width columns first — status (emoji), date,
  priority — then tag or name, then free-text description last. Keeps columns
  aligned in the human's editor.
- **When a bookkeeping loop repeats, script it.** Chris, 2026-08-29: *"are there
  any things you do repeatedly that could be ops/scripts"* — nine lanes × eight
  shell calls each turned out to be most of a context window spent on
  bookkeeping. The judgement stays human: a real code conflict, and what a lane's
  board substance should say.

### Fences are advisory, not exclusive (Chris, 2026-08-28)

Briefs enumerate the files or modules a lane may touch, and overlaps are
deliberate choices rather than accidents — but **do not gate a lane on finding
files nobody else touches.** Chris: *"we don't need to work on things that are
exclusively on free files — given we use workspaces we can sacrifice a little
cleanup pre-merge and have more stuff work in parallel, as long as there's not
massive refactors involved."*

Overlapping modules are fine; the coordinator pays for it in rebase conflicts,
which are cheap and local. What still warrants serialising is a **massive
refactor** — a lane rewriting a file's structure makes concurrent edits to that
file genuinely expensive, not merely annoying. Judge by *"is the file being
restructured, or just extended?"* — extension parallelises, restructuring does
not. Fences remain useful as **ownership hints** ("if you need to change X, say
so in your report") rather than hard walls.

### Don't mix long work into a field of short work (Chris, 2026-08-28)

*"Long work parallelizes poorly with lots of short work, unless it's something
really different like research."*

A multi-stage lane among many short lanes is the worst case: it holds a workspace
for hours, its base drifts under it while everything else lands, and its merge
grows monotonically with every sibling that finishes. Short lanes absorb rebases
cheaply; long ones pay compound interest. The exception is **work of a genuinely
different kind** — research, surveys, design docs, measurement rounds — which
parallelises fine at any length because it mostly touches docs and cannot collide
semantically with code lanes.

Rule of thumb: **fire long multi-stage code work when the board is quiet**, or
stage it explicitly so each stage is short. Prefer many small lanes otherwise.

### Lanes cannot address each other by label (learned 2026-08-29)

A lane cannot see another lane's workspace or wait on it, and a board label is
not an agent name. Two lanes that needed to agree on who owned a shared type
tried to message each other by label and the send bounced — **the coordinator is
the only one that holds every lane's agent id**, so every cross-lane message
routes through it whether or not that was intended. Cross-lane dependencies are
the briefer's problem.

- **Put the peer's agent id in the brief** when two lanes are known in advance to
  share a file or an abstraction. One line, and it saves a bounce and a round
  trip at exactly the moment the two lanes are about to write the same code.
- **Relay promptly, and decide while relaying.** A relayed message is not a
  postal service — the coordinator is the only party who can see both lanes, so a
  request like *"we both think we own this type"* comes back **settled**, not
  forwarded. The cost of hesitating is that both lanes keep working, which is
  precisely the failure this is meant to prevent.

The deeper point, proved by two lanes in the same area independently building the
same enum: **overlapping lanes are cheap for edits and expensive for
abstractions, because only one abstraction can live.** When a decision lands
mid-flight that implies a refactor, that refactor is a *new shared abstraction*
by definition — assign it to exactly one lane, in writing, before either starts.
The rule of thumb that settled it: **the lane that cannot do its own work without
touching those lines owns them.**

### Show the UI; don't make the human rebuild it (Chris, 2026-08-29)

A UI change is rendered, not described — into a shared, dated, lane-labelled
directory under the repo root, and said so in the report.

The reason is arithmetic. Chris: *"having to rebuild it from scratch myself takes
a while … and subagents are already doing all that work and screenshotting."* The
lane has already paid the build cost and already rendered the scenes to check its
own work; without redirection those images land in a build directory and are
deleted by the next clean. Redirecting them costs one environment variable and
saves a full rebuild per lane.

- **The path is the repo root's, not `$PWD`.** A lane runs inside its own
  workspace, so `$PWD/…` writes where the human never looks and which is deleted
  with the workspace. Five lanes rendered 130 images into their own workspaces
  before this was caught (Chris: *"I don't see that screenshot"* … *"subagents
  should be making sure to write in .screenshots of the root project"*). Use an
  absolute path.
- **Keep only the scenes that changed.** Rendering is all-or-nothing, but a lane
  that copies every scene into its directory buries the two that matter, and five
  lanes doing it produced 130 near-identical files (Chris: *"why so many
  duplicate screenshots?!"*). Render to scratch, diff against the previous run,
  keep what changed plus what the lane added. The exception is a deliberate
  **baseline** set, which keeps everything precisely so the next lane has
  something to diff against.
- **Say both numbers** — *"21 rendered, 3 changed"*. "Nothing changed" is
  information too, and it is the sentence that tells the human he need not look.
- **`reference/` is evidence: never overwrite or clear it.** It holds captures of
  other people's software and the human's own manual captures. `progress/` is
  dated and lane-labelled so two lanes do not clobber each other, and stale
  directories there are free to delete — that is the difference.
- **A screenshot is not proof the change is good.** This is how the human sees a
  visual change without rebuilding; the measured claims still belong in the
  report and the changelog account.

### Briefing a lane

A lane is only as good as its brief. A good one carries: the constraints, the
**collisions it will hit** and who owns what, the APIs that moved under it since
it last ran, what you want in the report — and **which of the briefer's own
assumptions it should verify rather than trust**. That last one has repeatedly
returned the most valuable finding in a lane's report.

### Local-model delegation

Burn bulk-implementation tokens on a LOCAL model so the frontier-model quota goes
to judgment. Limits are real — three agents died to one in a single day.

Verified working: `pi --provider ollama --model <local-model> --no-session -p "…"`
(pi 0.84.3, headless print mode; `--mode json` for structured output,
`--session`/`-c` to iterate with context, e.g. feeding compile errors back).

**The tier**: human → coordinator → supervising agents (one per lane) → local
model (implementation batches).

**Supervisor protocol**, for an agent on delegable work:

1. Write the spec and the ACCEPTANCE TESTS first — the tests are the contract, so
   author them yourself and never delegate the tests that gate the delegate.
2. Delegate one tight batch: a narrow prompt naming exact files, conventions
   (point it at a model file to imitate), and the fence.
3. Verify: run the gates and review the whole diff. The local model **never
   commits**; the supervisor commits after review, pruning slop.
4. Iterate with the error output, at most ~2 rounds, then take over yourself —
   review cost must stay below writing cost.

**Delegate**: template-following module batches, test boilerplate, mechanical API
sweeps (e.g. a 45-file signature migration), doc plumbing.
**Never delegate**: architecture, compiler or unsafe internals, conflict
resolution, contract and design decisions, anything spec-ambiguous.

Policy, calibrated by the first pilot's failure (a history-test port: >10 min on
the local model, misused private APIs, wrong semantics, rewritten):

- Delegate ONLY well-specified mechanical work — rote sweeps with an exemplar
  ("here is one converted module, convert these N the same way"), test
  boilerplate from a template, doc-table regeneration, rename chores.
- Every delegated task ships with exact files, one worked example, the command to
  verify, and a hard instruction to change nothing else. Review the diff like a
  hostile PR; rewriting more than 30% means the task was miscategorised — do it
  directly next time and note it.
- Laptop heat and GPU contention are real: if the human needs the machine he says
  so and the local model pauses. Prefer batching runs over keeping the server
  warm.

The switch has been thrown both ways — paused for laptop heat, then **re-enabled
under quota pressure** (Chris, 2026-08-28, later the same day) when frontier
quota was near its limit for several days, exactly the trade the pause clause
anticipated: local latency beats no quota. Which way it is set *now* is live
state and belongs on the board, not here.

### Constraint hygiene (added 2026-08-28 after a real miss)

When work is throttled by a constraint — quota, a blocking lane, a paused
dependency — record WHICH constraint and re-check it when the situation changes.
A throttle set for reason X must be lifted the moment X stops applying; the
failure mode is a conservative policy outliving its cause and quietly serialising
work nobody chose to serialise. Concretely: a model-quota freeze correctly
stopped lanes on that model, but lanes on a different model were never subject to
it, and after the switch the freeze should have been lifted proactively rather
than waiting for the human to ask why everything was sequential. Default posture
remains parallelise-and-merge-later.

---

## 5. How we state claims — #SimplyAILogic (Chris, 2026-08-28)

Docs, reports and briefs reason **modally and constructively**, not classically.
Four operational habits:

1. **No excluded middle.** Don't force a binary. "Not yet constructed" is a real
   third state — a design isn't right-or-wrong before it exists, and a claim
   isn't true-or-false before someone builds the evidence.
2. **Distinguish kinds of impossibility, because the distinction is actionable.**
   ⚫️ = impossible in every world. 🚫 = impossible *in this context*, open
   elsewhere. ◇ = possible. □ = necessary. **Most "we can't do that" is 🚫**, and
   naming it points at the context that would have to change — usually a single
   mechanism is the only thing making it impossible, and saying which one is the
   lever. Writing ⚫️ where 🚫 is meant hides that lever.
3. **Belief carries evidence and stays revisable.** `(p ∧ e) → Bp`: assert with
   what produced the assertion. There is **no T axiom for belief** — we hold
   false beliefs, and several have been corrected the day they were written. On
   contradiction, revise (`Bp ∧ B¬p → BR`) rather than pick a side or stall. Keep
   the correction visible; it is usually more useful than the claim.
4. **Fixed past, open future** (`□Past`, `◇Future`). Shipped work is stated
   flatly; roadmap and queue entries are **possibilities, not commitments**, and
   should read that way.

---

## 6. The big question (periodic review ritual)

When the human asks "the big question" — or at any natural lull — run the
pipeline review across the doc web, in this order:

1. **What other ideas could we be looking into?** — generate and file new rows in
   the ideas inbox (generalisations, gaps, adjacent envelope-pushes).
2. **What ideas can we start to put into the roadmap and spec out?** — promote
   mature rows: design rounds for the big ones, roadmap entries for the committed.
3. **What roadmap things can we pull onto the board?** — turn roadmap scope into
   launch-ready queue entries, lanes permitting.
4. **What board items can we conclude and mark done in the roadmap?** — close
   finished lanes in both places; prune the queue of superseded entries.
5. **What decisions can we be discussing?** — surface open calls (pending
   reviews, design forks, go/no-go gates) with a recommendation each.

Answer all five explicitly, docs updated as you go (docs before eng).

---

## 7. Recovery (crashed session / fresh context)

1. Read the project's `AGENTS.md`, this file, and the board.
2. `jj log` — every lane's chain is committed incrementally; compare with the
   board's in-flight table. `jj workspace list` plus a per-workspace `jj status`
   finds uncommitted lane work.
3. Finished unmerged chains → hand to a `integrator`. Dead agents → relaunch with
   the fence from the board row; the chain survives.
4. **Salvage dead agents' transcripts.** A killed subagent's full transcript
   survives at
   `~/.claude/projects/<project>/<session>/subagents/agent-<id>.jsonl` even when
   the harness says it "won't be resumed". Give the relaunched agent its
   predecessor's transcript path — the tail holds the exact plan and next step it
   was executing, which is finer-grained than the committed workspace state.
   (Learned 2026-08-28: three fresh agents were launched from workspace state
   alone; the transcripts had survived.)
5. Private memory duplicates the key conventions and points back here.
