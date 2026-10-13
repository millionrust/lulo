#!/usr/bin/env python3
"""Writing Tools in Text Editor and Notes, end to end, in a private session.

ADR 0024 phase 2 (Writing Tools). A private headless Sway + nested niri
shell (the parallel journeys' session), a private session bus that
D-Bus-activates `rmac-intelligence-service`, and private XDG directories.
The model is the fixture engine (`RMAC_INTELLIGENCE_ENGINE=fixture`): CI has
no model file. It writes a fixed answer one word at a time
(`RMAC_INTELLIGENCE_FIXTURE_DELAY_MS` apart), so the answer visibly streams
and can be stopped halfway.

1. Lulo Intelligence **off**: Text Editor's Edit menu (its `AppMenu2`
   layout, which the menu bar draws) has no Writing Tools, activating
   Proofread is refused, and the service never starts.
2. **On**: Edit ▸ Writing Tools is there with all nine commands, in Text
   Editor and in Notes. Proofread on a typed paragraph opens the panel, the
   answer streams in (several partial answers seen, first delta well before
   the end), Replace puts it in the document, and ⌘Z gives the original
   back.
3. **Cancel**: Make Friendly, then Escape after the first words: the panel
   closes, nothing more arrives, and the document is untouched.
4. The text's right-click menu shows Writing Tools (recorded).
5. The service exits by itself once idle.

    python3 scripts/behavior/run_writing_tools.py --bin-dir DIR --niri NIRI
"""

from __future__ import annotations

import argparse
import fcntl
import json
from pathlib import Path
import subprocess
import sys
import tempfile
import time

HERE = Path(__file__).resolve().parent
sys.path.insert(0, str(HERE))
sys.path.insert(0, str(HERE.parent / "parallel"))
import run_lulo  # noqa: E402
import run_lulo_journey  # noqa: E402
import run_spotlight_intents as intents  # noqa: E402

SERVICE = "rmac-intelligence-service"
IDLE_SECONDS = 4
# 150 ms a word: an 11-word answer takes well over a second to stream.
DELAY_MS = 150
TEXT_EDITOR_MENU = "org.rmac.TextEditor.Menu"
NOTES_MENU = "org.rmac.Notes.Menu"
MENU_PATH = "/org/rmac/AppMenu1"
WRITING_TOOLS = [
    "Show Writing Tools", "Proofread", "Rewrite", "Make Friendly", "Make Professional",
    "Make Concise", "Summarise", "Create Key Points", "Make List",
]
# The fixture's Proofread (crates/rmac-intelligence-service/src/fixture.rs).
TYPED = "i think teh cat is wierd. it sat on teh mat and slept."
PROOFREAD = "I think the cat is weird. It sat on the mat and slept."
FRIENDLY_START = "Hi!"


class Scenario(intents.BaseScenario):
    def __init__(self, args, work: Path):
        super().__init__(args, work)
        self.driver.data["title"] = "Writing Tools"
        self.errors: list[str] = []
        self.trace = work / "text-editor-frame-trace.csv"
        self.extra_env: dict[str, str] = {}
        self.watcher = intents.TraceWatcher(self.trace)

    # ---- helpers ---------------------------------------------------------

    def gdbus(self, destination: str, method: str, *arguments: str) -> subprocess.CompletedProcess:
        return subprocess.run(
            ["gdbus", "call", "--session", "--dest", destination, "--object-path", MENU_PATH,
             "--method", method, *arguments],
            env=self.driver.session.env, capture_output=True, text=True, timeout=20)

    def layout(self, destination: str) -> str:
        result = self.gdbus(destination, "org.rmac.AppMenu2.Layout")
        return result.stdout if result.returncode == 0 else ""

    def wait_layout(self, destination: str, timeout: float = 20) -> str:
        return intents.wait_until(lambda: self.layout(destination), timeout, 0.5) or ""

    def activate(self, action: str) -> subprocess.CompletedProcess:
        return self.gdbus(TEXT_EDITOR_MENU, "org.rmac.AppMenu2.Activate", action)

    def has_writing_tools(self, layout: str) -> dict[str, bool]:
        return {label: f"'{label}'" in layout for label in ["Writing Tools", *WRITING_TOOLS]}

    def app_pid(self, app: str) -> int | None:
        process = self.driver.apps.get(app)
        return process.pid if process is not None else None

    def nodes(self, app: str):
        import pyatspi

        for node, _pid in self.driver.accessible(self.app_pid(app)):
            try:
                if node.getState().contains(pyatspi.STATE_SHOWING):
                    yield node
            except Exception:  # noqa: BLE001
                continue

    def panel_open(self) -> bool:
        for node in self.nodes("text-editor"):
            try:
                if node.name == "Writing Tools" and node.getRoleName() in {"dialog", "panel", "alert"}:
                    return True
            except Exception:  # noqa: BLE001
                continue
        return False

    def panel_text(self, start: str) -> str | None:
        """The panel's answer (a label named with its text) starting with
        `start`."""
        for node in self.nodes("text-editor"):
            try:
                name = node.name or ""
                if name.startswith(start) and node.getRoleName() not in {
                        "text", "entry", "document text", "paragraph"}:
                    return name
            except Exception:  # noqa: BLE001
                continue
        return None

    def panel_nodes(self) -> list[tuple[str, str]]:
        """(role, name, showing) of every named node, for a failure's report."""
        import pyatspi

        out = []
        for node, _pid in self.driver.accessible(self.app_pid("text-editor")):
            try:
                if node.name:
                    out.append((node.getRoleName(), node.name[:80],
                                node.getState().contains(pyatspi.STATE_SHOWING)))
            except Exception:  # noqa: BLE001
                continue
        return out

    def document_text(self) -> str:
        """The document body's text, from its accessible text."""
        best = ""
        for node in self.nodes("text-editor"):
            try:
                if node.getRoleName() in {"label", "push button", "menu item", "dialog", "status bar"}:
                    continue
                text = self.driver.text_of(node)
            except Exception:  # noqa: BLE001
                continue
            if ("cat" in text or "mat" in text) and len(text) > len(best):
                best = text
        return best

    def key(self, chord: str) -> None:
        self.driver.session.pointer.key(chord)
        time.sleep(0.3)

    # ---- the scenario ----------------------------------------------------

    def launch_text_editor(self) -> None:
        self.driver.launch({"launch": "text-editor"})
        # Each launch truncates its frame trace (the window is open, so the
        # new file exists): follow the new one from its start.
        self.watcher = intents.TraceWatcher(self.trace)
        layout = self.wait_layout(TEXT_EDITOR_MENU)
        if not layout:
            raise RuntimeError("Text Editor never published its menus")
        time.sleep(1.5)

    def quit(self, app: str) -> None:
        process = self.driver.apps.pop(app, None)
        if process is not None and process.poll() is None:
            process.terminate()
            try:
                process.wait(10)
            except subprocess.TimeoutExpired:
                process.kill()
        time.sleep(1)

    def off(self) -> None:
        intents.set_enabled(self.config_home, False)
        self.launch_text_editor()
        # The first look at the setting happens off the UI thread.
        time.sleep(2)
        shown = self.has_writing_tools(self.layout(TEXT_EDITOR_MENU))
        self.facts["off_menu"] = shown
        if shown["Writing Tools"]:
            self.errors.append("Edit ▸ Writing Tools was shown while Lulo Intelligence was off")
        refused = self.activate("writing_tools::Proofread")
        self.facts["off_activate"] = (refused.returncode, refused.stderr.strip()[-200:])
        if refused.returncode == 0:
            self.errors.append("Proofread could be activated while Lulo Intelligence was off")
        time.sleep(2)
        self.facts["off_service_started"] = bool(intents.service_pids(self.service))
        if self.facts["off_service_started"]:
            self.errors.append("the service started while Lulo Intelligence was off")
        self.quit("text-editor")

    def type_paragraph(self) -> None:
        self.driver.session.pointer.type_text(TYPED)
        time.sleep(0.5)
        self.key("cmd-a")

    def proofread_replace_undo(self) -> None:
        self.type_paragraph()
        before = self.document_text()
        self.facts["typed"] = before
        if TYPED not in before:
            self.errors.append(f"the typed paragraph is not in the document: {before!r}")
        asked = time.monotonic()
        result = self.activate("writing_tools::Proofread")
        if result.returncode != 0:
            self.errors.append(f"Edit ▸ Writing Tools ▸ Proofread was refused: {result.stderr[-300:]}")
            return
        if not intents.wait_until(self.panel_open, 10, 0.2):
            self.errors.append("the Writing Tools panel did not open")
            return
        time.sleep(0.6)
        self.facts["showing_while_answering"] = self.panel_nodes()
        # Watch the answer stream in.
        partials: list[str] = []
        deadline = time.monotonic() + 30
        while time.monotonic() < deadline:
            text = self.panel_text("I think") or ""
            if text and (not partials or partials[-1] != text):
                partials.append(text)
            if text == PROOFREAD:
                break
            time.sleep(0.1)
        self.facts["proofread_partials"] = partials
        self.watcher.wait_for("writing_tools_done", asked, 10)
        marks = {event.split(":")[0]: moment for moment, event in self.watcher.events if moment >= asked}
        first, done = marks.get("writing_tools_first_delta"), marks.get("writing_tools_done")
        self.facts["proofread_first_delta_s"] = round(first - asked, 2) if first else None
        self.facts["proofread_done_s"] = round(done - asked, 2) if done else None
        if not partials or partials[-1] != PROOFREAD:
            self.errors.append(f"the panel never showed the whole answer: {partials[-3:]}")
        if len({len(partial) for partial in partials}) < 3:
            self.errors.append(f"the answer did not stream in (only {len(partials)} states seen)")
        if first is None or done is None or done - first < 0.5:
            self.errors.append(f"no streaming in the trace (first delta {first}, done {done})")
        before_replace = self.document_text()
        if before_replace != before:
            self.errors.append("the document changed before Replace")
        try:
            self.driver.click("Replace")
        except RuntimeError:
            self.key("return")
        replaced = intents.wait_until(lambda: PROOFREAD in self.document_text(), 8, 0.2)
        self.facts["after_replace"] = self.document_text()
        if not replaced:
            self.errors.append(f"Replace did not put the answer in: {self.facts['after_replace']!r}")
        if self.panel_open():
            self.errors.append("the panel stayed open after Replace")
        self.key("cmd-z")
        restored = intents.wait_until(lambda: TYPED in self.document_text(), 8, 0.2)
        self.facts["after_undo"] = self.document_text()
        if not restored:
            self.errors.append(f"⌘Z did not restore the original: {self.facts['after_undo']!r}")

    def cancel_midway(self) -> None:
        self.key("cmd-a")
        before = self.document_text()
        asked = time.monotonic()
        result = self.activate("writing_tools::MakeFriendly")
        if result.returncode != 0:
            self.errors.append(f"Make Friendly was refused: {result.stderr[-300:]}")
            return
        if not self.watcher.wait_for("writing_tools_first_delta", asked, 10):
            self.errors.append("Make Friendly never started answering")
            return
        time.sleep(0.4)
        partial = self.panel_text(FRIENDLY_START)
        self.facts["cancel_partial"] = partial
        self.key("escape")
        closed = intents.wait_until(lambda: not self.panel_open(), 5, 0.2)
        stopped_at = time.monotonic()
        self.facts["cancel_closed_panel"] = bool(closed)
        if not closed:
            self.errors.append("Escape did not close the panel mid-answer")
        time.sleep(3)
        after = self.document_text()
        self.facts["cancel_document"] = after
        if after != before:
            self.errors.append(f"cancelling changed the document: {after!r}")
        self.watcher.wait_for("writing_tools_cancelled", asked, 1)
        marks = [event for moment, event in self.watcher.events
                 if moment >= asked and event.startswith("writing_tools")]
        self.facts["cancel_marks"] = marks
        if "writing_tools_cancelled" not in marks:
            self.errors.append("the panel did not tell the service to stop")
        late = [event for moment, event in self.watcher.events
                if moment >= stopped_at and event.startswith("writing_tools_done")]
        if late:
            self.errors.append(f"an answer still finished after Stop: {late}")
        if partial and FRIENDLY_START not in partial:
            self.errors.append(f"unexpected partial answer {partial!r}")

    def context_menu(self) -> None:
        window = self.driver.window()
        if not window:
            return
        x, y, w, h = self.driver.session.geometry(window)
        px, py = self.driver.session.parent_point(x + w / 2, y + h / 2)
        self.driver.session.pointer.click(px, py, self.driver.session.parent_width,
                                          self.driver.session.parent_height, button="right")
        time.sleep(1)
        names = {node.name for node in self.nodes("text-editor") if node.name}
        self.facts["context_menu_writing_tools"] = "Writing Tools" in names
        self.facts["context_menu_cut"] = "Cut" in names
        self.key("escape")

    def tiny_tier(self) -> None:
        """On a PC that runs only the Tiny model, Summarise says it needs a
        faster PC and never asks the service (ADR 0024 phase 2 gating)."""
        self.extra_env = {"RMAC_INTELLIGENCE_FIXTURE_TIER": "tiny"}
        self.launch_text_editor()
        self.extra_env = {}
        self.type_paragraph()
        asked = time.monotonic()
        result = self.activate("writing_tools::Summarise")
        if result.returncode != 0:
            self.errors.append(f"Summarise could not be chosen: {result.stderr[-300:]}")
            return
        message = intents.wait_until(lambda: self.panel_text("Summarise needs"), 8, 0.2)
        self.facts["tiny_summarise"] = message
        if message != "Summarise needs a faster PC.":
            self.errors.append(f"Summarise on the Tiny model did not say it needs a faster PC: {message!r}")
        time.sleep(1)
        self.watcher.wait_for("writing_tools_asked", asked, 0.5)
        asked_marks = [event for moment, event in self.watcher.events
                       if moment >= asked and event == "writing_tools_asked"]
        if asked_marks:
            self.errors.append("Summarise on the Tiny model still asked the service")
        self.key("escape")
        self.quit("text-editor")

    def notes_menu(self) -> None:
        self.driver.launch({"launch": "notes"})
        layout = self.wait_layout(NOTES_MENU, 30)
        time.sleep(2)
        shown = self.has_writing_tools(self.layout(NOTES_MENU) or layout)
        self.facts["notes_menu"] = shown
        if not all(shown.values()):
            self.errors.append(f"Notes' Edit ▸ Writing Tools is incomplete: {shown}")
        self.quit("notes")

    def run(self) -> dict:
        original_spawn = self.driver.session.spawn

        def spawn_with_trace(argv, name, extra=None):
            if name == "text-editor":
                extra = {**(extra or {}), "RMAC_FRAME_TRACE": str(self.trace), **self.extra_env}
            return original_spawn(argv, name, extra)

        self.driver.session.spawn = spawn_with_trace
        self.driver.start()

        self.off()

        intents.set_enabled(self.config_home, True)
        self.launch_text_editor()
        shown = intents.wait_until(
            lambda: (lambda found: found if all(found.values()) else None)(
                self.has_writing_tools(self.layout(TEXT_EDITOR_MENU))), 10, 0.5)
        self.facts["on_menu"] = shown or self.has_writing_tools(self.layout(TEXT_EDITOR_MENU))
        if not shown:
            self.errors.append(f"Edit ▸ Writing Tools is incomplete: {self.facts['on_menu']}")
        else:
            self.proofread_replace_undo()
            self.cancel_midway()
            self.context_menu()
        self.quit("text-editor")
        self.tiny_tier()
        self.notes_menu()

        gone = intents.wait_until(lambda: not intents.service_pids(self.service), IDLE_SECONDS + 15, 0.5)
        self.facts["service_ran"] = "proofread_partials" in self.facts
        self.facts["service_exited_when_idle"] = bool(gone)
        if not gone:
            self.errors.append("the service was still running after its idle timeout")
        return {"facts": self.facts, "errors": self.errors}


def inner(args) -> int:
    work = Path(args.inner)
    scenario = Scenario(args, work)
    try:
        result = scenario.run()
    except Exception as error:  # noqa: BLE001 - a crash must fail, never pass
        result = {"facts": scenario.facts, "errors": scenario.errors + [f"{type(error).__name__}: {error}"]}
    finally:
        scenario.finish()
    status = "pass" if not result["errors"] else "fail"
    print(json.dumps({"scenario": "writing-tools", "status": status, **result}, indent=2,
                     default=str), flush=True)
    return 0 if status == "pass" else 1


def outer(args) -> int:
    lock = open("/tmp/lulo-journey.lock", "w")
    fcntl.flock(lock, fcntl.LOCK_EX)
    work = Path(tempfile.mkdtemp(prefix="lulo-writing-"))
    try:
        env = run_lulo.isolated_environment(work)
        run_lulo.refuse_live_session(env)
        bins = Path(args.bin_dir).expanduser().resolve()
        links = work / "bins"
        links.mkdir()
        # The service, and the apps that call it, are copied side by side, so
        # the caller check's "a Lulo program beside me" rule accepts them
        # wherever --bin-dir came from.
        copy_in_place = {SERVICE, "rmac-text-editor", "rmac-notes"}
        sources = {source.name: source for source in bins.iterdir()}
        if args.override_bin_dir:
            # Freshly built copies of just these programs; the rest of the
            # session (shell, menu bar) comes from --bin-dir.
            overrides = Path(args.override_bin_dir).expanduser().resolve()
            for name in copy_in_place:
                if (overrides / name).is_file():
                    sources[name] = overrides / name
        for source in sources.values():
            if source.is_file() and source.name not in {"dock", "mission-control"}:
                if source.name in copy_in_place:
                    target = links / source.name
                    target.write_bytes(source.read_bytes())
                    target.chmod(0o755)
                else:
                    (links / source.name).symlink_to(source)
        for alias, source in (("dock", "rmac-dock"), ("mission-control", "rmac-mission-control")):
            (links / alias).symlink_to(links / source)
        if not (links / SERVICE).is_file():
            raise SystemExit(f"{SERVICE} is not in {bins}")
        run_lulo.install_shortcut_dispatcher(env, links)
        config = run_lulo_journey.private_bus(work, env, links)
        (work / "dbus-services" / "org.rmac.Intelligence1.service").write_text(
            "[D-BUS Service]\nName=org.rmac.Intelligence1\n" f"Exec={links / SERVICE}\n")
        env["RMAC_INTELLIGENCE_ENGINE"] = "fixture"
        env["RMAC_INTELLIGENCE_FIXTURE_DELAY_MS"] = str(DELAY_MS)
        env["RMAC_INTELLIGENCE_IDLE_SECONDS"] = str(IDLE_SECONDS)
        command = ["dbus-run-session", f"--config-file={config}", "--", sys.executable,
                   str(Path(__file__).resolve()), "--inner", str(work), "--bin-dir", str(links),
                   "--niri", args.niri]
        return subprocess.call(command, env=env, close_fds=True)
    finally:
        if run_lulo.reap(work / "runtime"):
            time.sleep(1)
            run_lulo.reap(work / "runtime")
        if args.keep:
            print(f"kept {work}", flush=True)
        else:
            run_lulo.remove_tree(work)
        lock.close()


def main() -> int:
    parser = argparse.ArgumentParser(description=__doc__,
                                     formatter_class=argparse.RawDescriptionHelpFormatter)
    parser.add_argument("--bin-dir", required=True)
    parser.add_argument("--niri", default="/usr/bin/niri")
    parser.add_argument("--override-bin-dir",
                        help="take the service, Text Editor and Notes from here instead")
    parser.add_argument("--keep", action="store_true")
    parser.add_argument("--inner", help=argparse.SUPPRESS)
    parser.set_defaults(frame_only=False, full_too=False, dump_a11y=False)
    args = parser.parse_args()
    if args.inner:
        return inner(args)
    return outer(args)


if __name__ == "__main__":
    raise SystemExit(main())
