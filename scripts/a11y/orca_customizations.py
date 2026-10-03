"""orca-customizations.py for the automated Orca audit (scripts/a11y/orca_audit.py).

orca_audit.py copies this file into the private run's
$XDG_DATA_HOME/orca/, the one hook Orca loads before it starts. It never
reaches the owner's real Orca settings: the run's HOME and XDG_* point into
a temporary directory and GSettings uses the memory backend.

It makes Orca silent and inspectable:
  * no speech server: speech-dispatcher is never spawned, so nothing can
    reach a sound card;
  * every utterance Orca would have spoken is appended, as one JSON line,
    to $LULO_ORCA_SPEECH;
  * earcons and braille are switched off.
"""

import json
import os
import time

from orca import speech, speech_manager

_PATH = os.environ.get("LULO_ORCA_SPEECH")


def _record(kind, text):
    if not _PATH:
        return
    try:
        with open(_PATH, "a", encoding="utf-8") as handle:
            handle.write(json.dumps({"t": time.time(), "kind": kind, "text": str(text)}) + "\n")
    except OSError:
        pass


def _no_server(*_args, **_kwargs):
    return None


speech_manager.SpeechManager._init_server_from_module = staticmethod(_no_server)

_original_speak = speech._speak


def _speak(text, acss):
    _record("speech", text)
    _original_speak(text, acss)


speech._speak = _speak

_original_character = speech.speak_character


def _speak_character(character, acss=None, cap_style=None):
    _record("character", character)
    _original_character(character, acss, cap_style)


speech.speak_character = _speak_character

try:
    from orca import sound

    sound.Player.play = lambda self, item, interrupt=True: None
    sound.Player.init = lambda self: None
except Exception:  # noqa: BLE001 - an older Orca without earcons
    pass

try:
    from orca import braille

    braille.init = lambda callback=None: False
except Exception:  # noqa: BLE001
    pass

_record("ready", "customizations loaded")
