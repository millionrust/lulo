#!/usr/bin/env python3
"""Verify staged or installed rmac application metadata and runtime links."""

from __future__ import annotations

import argparse
import configparser
import hashlib
import json
import os
from pathlib import Path
import re
import shutil
import stat
import subprocess
import xml.etree.ElementTree as ET


MANIFEST = Path("usr/share/rmac/application-package-manifest.json")
MAX_MANIFEST_BYTES = 128 * 1024
MAX_METADATA_BYTES = 1024 * 1024
XML_LANG = "{http://www.w3.org/XML/1998/namespace}lang"
TEXT_MIME_TYPES = (
    "text/plain",
    "text/markdown",
    "text/x-markdown",
    "text/rtf",
    "application/rtf",
)
# Preview claims only the formats it decodes (the image crate's PNG, JPEG,
# GIF, WebP, BMP and TIFF, and PDF through poppler), not every image/*.
PREVIEW_MIME_TYPES = (
    "application/pdf",
    "image/png",
    "image/jpeg",
    "image/gif",
    "image/webp",
    "image/bmp",
    "image/tiff",
)
# Archive Utility claims the archives rmac-archive expands (shared-mime-info
# names, including the older bzip spellings still in use).
ARCHIVE_MIME_TYPES = (
    "application/zip",
    "application/x-tar",
    "application/x-compressed-tar",
    "application/x-bzip2-compressed-tar",
    "application/x-bzip-compressed-tar",
    "application/x-xz-compressed-tar",
    "application/gzip",
    "application/x-bzip2",
    "application/x-bzip",
    "application/x-xz",
)
# Media Player claims the formats its libmpv build plays and the open panel
# offers; rmac-mimeapps.list makes it the rmac default for them.
PLAYER_MIME_TYPES = (
    "audio/mpeg",
    "audio/mp4",
    "audio/x-m4a",
    "audio/aac",
    "audio/flac",
    "audio/x-flac",
    "audio/ogg",
    "audio/opus",
    "audio/x-vorbis+ogg",
    "audio/x-wav",
    "audio/wav",
    "video/mp4",
    "video/x-m4v",
    "video/quicktime",
    "video/x-matroska",
    "video/webm",
    "video/x-msvideo",
    "video/mpeg",
    "video/ogg",
)
MIMEAPPS = Path("usr/share/applications/rmac-mimeapps.list")
SUPERSEDED_APPS = Path("usr/share/rmac/superseded-apps.list")
# Kept in sync with DEFAULT_SUPERSEDED in crates/rmac-apps/src/superseded.rs.
# The key is the superseded desktop entry; the value is the rmac identity
# (from APPLICATIONS below) that does the same job.
SUPERSEDED_APPLICATIONS = {
    "org.gnome.Nautilus.desktop": "org.rmac.Files",
    "org.gnome.TextEditor.desktop": "org.rmac.TextEditor",
    "gnome-system-monitor.desktop": "org.rmac.SystemMonitor",
    "org.gnome.SystemMonitor.desktop": "org.rmac.SystemMonitor",
    "gnome-calculator.desktop": "org.rmac.Calculator",
    "org.gnome.Calculator.desktop": "org.rmac.Calculator",
    "org.gnome.Loupe.desktop": "org.rmac.Preview",
    "org.gnome.Evince.desktop": "org.rmac.Preview",
    "org.gnome.Papers.desktop": "org.rmac.Preview",
    "org.gnome.Terminal.desktop": "org.rmac.Terminal",
    "org.gnome.Ptyxis.desktop": "org.rmac.Terminal",
    "foot.desktop": "org.rmac.Terminal",
    "footclient.desktop": "org.rmac.Terminal",
    "foot-server.desktop": "org.rmac.Terminal",
    "org.gnome.clocks.desktop": "org.rmac.Clock",
    "org.gnome.Weather.desktop": "org.rmac.Weather",
}
APPLICATIONS = {
    "org.rmac.AppDrawer": {
        "name": "Apps",
        "generic": "Application Launcher",
        "summary": "Browse and launch installed applications",
        "keywords": "applications;apps;launcher;programs;",
        "binary": "rmac-app-drawer",
        "categories": "System;",
        "hidden": False,
    },
    "org.rmac.ArchiveUtility": {
        "name": "Archive Utility",
        "generic": "Archive Manager",
        "summary": "Expand zip and tar archives",
        "keywords": "archive;zip;tar;expand;uncompress;",
        "binary": "rmac-archive-utility",
        "categories": "Utility;Archiving;Compression;",
        "hidden": True,
    },
    "org.rmac.Calculator": {
        "name": "Calculator",
        "generic": "Calculator",
        "summary": "Perform basic arithmetic calculations",
        "keywords": "calculator;math;arithmetic;numbers;",
        "binary": "rmac-calculator",
        "categories": "Utility;Calculator;",
        "hidden": False,
    },
    "org.rmac.Clock": {
        "name": "Clock",
        "generic": "Clock",
        "summary": "World clock, alarms, stopwatch and timers",
        "keywords": "clock;alarm;timer;stopwatch;time;world;",
        "binary": "rmac-clock",
        "categories": "Utility;Clock;",
        "hidden": False,
    },
    "org.rmac.Files": {
        "name": "Files",
        "generic": "File Manager",
        "summary": "Browse and organize files and folders",
        "keywords": "finder;files;folders;storage;browse;",
        "binary": "rmac-files",
        "categories": "System;FileTools;FileManager;",
        "hidden": False,
    },
    "org.rmac.Notes": {
        "name": "Notes",
        "generic": "Note Taking",
        "summary": "Write and organize private local notes",
        "keywords": "notes;writing;lists;organize;",
        "binary": "rmac-notes",
        "categories": "Office;",
        "hidden": False,
    },
    "org.rmac.Player": {
        "name": "Media Player",
        "generic": "Media Player",
        "summary": "Play audio and video files",
        "keywords": "video;audio;music;movie;player;media;",
        "binary": "rmac-player",
        "categories": "AudioVideo;Player;Audio;Video;",
        "hidden": False,
    },
    "org.rmac.Preview": {
        "name": "Preview",
        "generic": "Document Viewer",
        "summary": "View images and PDF documents",
        "keywords": "preview;image;photo;pdf;viewer;",
        "binary": "rmac-preview",
        "categories": "Graphics;Viewer;",
        "hidden": False,
    },
    "org.rmac.SystemMonitor": {
        "name": "System Monitor",
        "generic": "Process Viewer",
        "summary": "Inspect processes and system resource use",
        "keywords": "process;cpu;memory;disk;network;",
        "binary": "rmac-system-monitor",
        "categories": "System;Monitor;",
        "hidden": False,
    },
    "org.rmac.SystemSettings": {
        "name": "Settings",
        "generic": "System Settings",
        "summary": "Configure the desktop, devices, and system",
        "keywords": "settings;preferences;configuration;devices;",
        "binary": "rmac-system-settings",
        "categories": "Settings;DesktopSettings;",
        "hidden": False,
    },
    "org.rmac.Terminal": {
        "name": "Terminal",
        "generic": "Terminal Emulator",
        "summary": "Use the command line in a native terminal",
        "keywords": "shell;prompt;command;console;",
        "binary": "rmac-terminal",
        "categories": "System;TerminalEmulator;",
        "hidden": False,
    },
    "org.rmac.TextEditor": {
        "name": "Text Editor",
        "generic": "Text Editor",
        "summary": "Create and edit plain text documents",
        "keywords": "text;editor;document;markdown;",
        "binary": "rmac-text-editor",
        "categories": "Utility;TextEditor;",
        "hidden": False,
    },
    "org.rmac.Weather": {
        "name": "Weather",
        "generic": "Weather Forecast",
        "summary": "Current conditions and forecasts for chosen cities",
        "keywords": "weather;forecast;temperature;rain;climate;",
        "binary": "rmac-weather",
        "categories": "Utility;",
        "hidden": False,
    },
}
HINDI = {
    "Apps": "ऐप्स",
    "Application Launcher": "अनुप्रयोग लॉन्चर",
    "Browse and launch installed applications": "इंस्टॉल किए गए अनुप्रयोग देखें और चलाएँ",
    "applications;apps;launcher;programs;": "अनुप्रयोग;ऐप्स;लॉन्चर;प्रोग्राम;",
    "Files": "फ़ाइलें",
    "File Manager": "फ़ाइल प्रबंधक",
    "Browse and organize files and folders": "फ़ाइलें और फ़ोल्डर देखें और व्यवस्थित करें",
    "finder;files;folders;storage;browse;": "फ़ाइंडर;फ़ाइलें;फ़ोल्डर;स्टोरेज;ब्राउज़;",
    "Notes": "नोट्स",
    "Note Taking": "नोट लिखना",
    "Write and organize private local notes": "निजी स्थानीय नोट लिखें और व्यवस्थित करें",
    "notes;writing;lists;organize;": "नोट्स;लेखन;सूचियाँ;व्यवस्थित;",
    "System Monitor": "सिस्टम मॉनिटर",
    "Process Viewer": "प्रक्रिया दर्शक",
    "Inspect processes and system resource use": "प्रक्रियाओं और सिस्टम संसाधनों के उपयोग की जाँच करें",
    "process;cpu;memory;disk;network;": "प्रक्रिया;सीपीयू;मेमोरी;डिस्क;नेटवर्क;",
    "Settings": "सेटिंग्स",
    "System Settings": "सिस्टम सेटिंग्स",
    "Configure the desktop, devices, and system": "डेस्कटॉप, उपकरण और सिस्टम कॉन्फ़िगर करें",
    "settings;preferences;configuration;devices;": "सेटिंग्स;प्राथमिकताएँ;कॉन्फ़िगरेशन;उपकरण;",
    "Terminal": "टर्मिनल",
    "Terminal Emulator": "टर्मिनल एमुलेटर",
    "Use the command line in a native terminal": "मूल टर्मिनल में कमांड लाइन का उपयोग करें",
    "shell;prompt;command;console;": "शेल;प्रॉम्प्ट;कमांड;कंसोल;",
    "Text Editor": "पाठ संपादक",
    "Create and edit plain text documents": "सादा पाठ दस्तावेज़ बनाएँ और संपादित करें",
    "text;editor;document;markdown;": "पाठ;संपादक;दस्तावेज़;मार्कडाउन;",
    "New Document": "नया दस्तावेज़",
    "Calculator": "कैलकुलेटर",
    "Perform basic arithmetic calculations": "बुनियादी अंकगणितीय गणनाएँ करें",
    "calculator;math;arithmetic;numbers;": "कैलकुलेटर;गणित;अंकगणित;संख्याएँ;",
    "Preview": "प्रीव्यू",
    "Document Viewer": "दस्तावेज़ दर्शक",
    "View images and PDF documents": "छवियाँ और PDF दस्तावेज़ देखें",
    "preview;image;photo;pdf;viewer;": "प्रीव्यू;छवि;फ़ोटो;पीडीएफ़;दर्शक;",
    "Archive Utility": "आर्काइव यूटिलिटी",
    "Archive Manager": "आर्काइव प्रबंधक",
    "Expand zip and tar archives": "zip और tar आर्काइव खोलें",
    "archive;zip;tar;expand;uncompress;": "आर्काइव;ज़िप;टार;विस्तार;अनकंप्रेस;",
    "Clock": "घड़ी",
    "World clock, alarms, stopwatch and timers": "विश्व घड़ी, अलार्म, स्टॉपवॉच और टाइमर",
    "clock;alarm;timer;stopwatch;time;world;": "घड़ी;अलार्म;टाइमर;स्टॉपवॉच;समय;विश्व;",
    "Weather": "मौसम",
    "Weather Forecast": "मौसम पूर्वानुमान",
    "Current conditions and forecasts for chosen cities": "चुने गए शहरों का वर्तमान मौसम और पूर्वानुमान",
    "weather;forecast;temperature;rain;climate;": "मौसम;पूर्वानुमान;तापमान;बारिश;जलवायु;",
    "Media Player": "मीडिया प्लेयर",
    "Play audio and video files": "ऑडियो और वीडियो फ़ाइलें चलाएँ",
    "video;audio;music;movie;player;media;": "वीडियो;ऑडियो;संगीत;फ़िल्म;प्लेयर;मीडिया;",
}
LOCALIZATION_FILES = {
    "LINGUAS",
    "POTFILES.in",
    "README.md",
    "hi.po",
    "rmac-apps.pot",
}


def _expected_paths() -> set[Path]:
    paths = {
        Path("usr/share/doc/rmac-apps/LICENSES.md"),
        Path("usr/share/doc/rmac-apps/copyright"),
        MIMEAPPS,
        SUPERSEDED_APPS,
    }
    for identity in APPLICATIONS:
        paths.update(
            {
                Path(f"usr/share/applications/{identity}.desktop"),
                Path(f"usr/share/icons/hicolor/scalable/apps/{identity}.svg"),
                Path(f"usr/share/metainfo/{identity}.metainfo.xml"),
            }
        )
    paths.update(
        Path("usr/share/doc/rmac-apps/localization") / filename
        for filename in LOCALIZATION_FILES
    )
    return paths


EXPECTED_PATHS = _expected_paths()


class VerificationError(RuntimeError):
    """A privacy-safe application package verification failure."""


def _regular_bytes(path: Path, maximum: int | None = None) -> tuple[bytes, int]:
    try:
        metadata = path.lstat()
    except OSError as error:
        raise VerificationError(f"required file is unavailable: {path.name}") from error
    if path.is_symlink() or not stat.S_ISREG(metadata.st_mode):
        raise VerificationError(f"required path is not a regular file: {path.name}")
    if maximum is not None and metadata.st_size > maximum:
        raise VerificationError(f"required file is too large: {path.name}")
    try:
        contents = path.read_bytes()
    except OSError as error:
        raise VerificationError(f"required file cannot be read: {path.name}") from error
    if len(contents) != metadata.st_size:
        raise VerificationError(f"required file changed while reading: {path.name}")
    return contents, stat.S_IMODE(metadata.st_mode)


def _load_manifest(root: Path) -> list[dict[str, str]]:
    raw, mode = _regular_bytes(root / MANIFEST, MAX_MANIFEST_BYTES)
    if mode != 0o644:
        raise VerificationError("application package manifest has the wrong mode")
    try:
        document = json.loads(raw)
    except (UnicodeDecodeError, json.JSONDecodeError) as error:
        raise VerificationError("application package manifest is invalid") from error
    if (
        not isinstance(document, dict)
        or document.get("format") != 1
        or type(document.get("format")) is not int
        or document.get("package") != "rmac-apps"
        or document.get("preserves_user_data") is not True
        or not isinstance(document.get("files"), list)
    ):
        raise VerificationError("application package manifest identity is invalid")
    entries = document["files"]
    paths = [entry.get("path") for entry in entries if isinstance(entry, dict)]
    if len(paths) != len(entries) or paths != sorted(paths) or len(paths) != len(set(paths)):
        raise VerificationError("application package manifest paths are not canonical")
    return entries


def _safe_manifest_path(value: object) -> Path:
    if not isinstance(value, str) or not value.startswith("/usr/share/"):
        raise VerificationError("application package manifest contains a non-share path")
    relative = Path(value.removeprefix("/"))
    if ".." in relative.parts or f"/{relative.as_posix()}" != value:
        raise VerificationError("application package manifest contains an unsafe path")
    return relative


def _tree_entries(root: Path) -> set[Path]:
    entries: set[Path] = set()

    def failed(error: OSError) -> None:
        raise VerificationError("application package tree cannot be inspected") from error

    for current, directories, filenames in os.walk(
        root, followlinks=False, onerror=failed
    ):
        current_path = Path(current)
        for directory in list(directories):
            path = current_path / directory
            entries.add(path.relative_to(root))
            if path.is_symlink():
                directories.remove(directory)
        for filename in filenames:
            entries.add((current_path / filename).relative_to(root))
    return entries


def _with_parent_directories(files: set[Path]) -> set[Path]:
    entries = set(files)
    for path in files:
        entries.update(parent for parent in path.parents if parent != Path("."))
    return entries


def _desktop_entry(path: Path) -> configparser.ConfigParser:
    raw, _mode = _regular_bytes(path, MAX_METADATA_BYTES)
    if b"DBusActivatable" in raw or b"StartupNotify" in raw:
        raise VerificationError(f"desktop entry makes an unproven claim: {path.name}")
    parser = configparser.ConfigParser(interpolation=None, strict=True)
    parser.optionxform = str
    try:
        parser.read_string(raw.decode("utf-8"))
    except (UnicodeDecodeError, configparser.Error) as error:
        raise VerificationError(f"desktop entry is invalid: {path.name}") from error
    return parser


def _localized(element: ET.Element, tag: str, language: str | None) -> str | None:
    for candidate in element.findall(tag):
        if candidate.attrib.get(XML_LANG) == language:
            return candidate.text
    return None


def _verify_desktop(root: Path, identity: str, specification: dict[str, object]) -> None:
    parser = _desktop_entry(root / f"usr/share/applications/{identity}.desktop")
    expected_sections = {"Desktop Entry"}
    if identity == "org.rmac.TextEditor":
        expected_sections.add("Desktop Action NewDocument")
    if set(parser.sections()) != expected_sections:
        raise VerificationError(f"desktop entry groups are invalid: {identity}")
    entry = parser["Desktop Entry"]
    binary = str(specification["binary"])
    expected_exec = f"/usr/bin/{binary}"
    if identity in (
        "org.rmac.TextEditor",
        "org.rmac.Preview",
        "org.rmac.ArchiveUtility",
        "org.rmac.Player",
    ):
        expected_exec += " %F"
    elif identity == "org.rmac.Files":
        # The session's folder handler: folders and file:// URIs, one window
        # each, as Finder opens them.
        expected_exec += " %U"
    required = {
        "Version": "1.5",
        "Type": "Application",
        "Name": specification["name"],
        "Name[hi]": HINDI[str(specification["name"])],
        "GenericName": specification["generic"],
        "GenericName[hi]": HINDI[str(specification["generic"])],
        "Comment": specification["summary"],
        "Comment[hi]": HINDI[str(specification["summary"])],
        "Keywords": specification["keywords"],
        "Keywords[hi]": HINDI[str(specification["keywords"])],
        "TryExec": f"/usr/bin/{binary}",
        "Exec": expected_exec,
        "Icon": identity,
        "Terminal": "false",
        "Categories": specification["categories"],
        "StartupWMClass": identity,
    }
    for key, value in required.items():
        if entry.get(key) != value:
            raise VerificationError(f"desktop entry field is invalid: {identity} {key}")
    if (entry.get("NoDisplay") == "true") is not bool(specification["hidden"]):
        raise VerificationError(f"desktop entry visibility is invalid: {identity}")

    if identity == "org.rmac.TextEditor":
        if entry.get("MimeType") != ";".join(TEXT_MIME_TYPES) + ";":
            raise VerificationError("Text Editor MIME declarations are invalid")
        if entry.get("Actions") != "NewDocument;":
            raise VerificationError("Text Editor action inventory is invalid")
        action = parser["Desktop Action NewDocument"]
        if dict(action) != {
            "Name": "New Document",
            "Name[hi]": HINDI["New Document"],
            "Exec": "/usr/bin/rmac-text-editor --new-document",
        }:
            raise VerificationError("Text Editor new-document action is invalid")
    elif identity == "org.rmac.Preview":
        if entry.get("MimeType") != ";".join(PREVIEW_MIME_TYPES) + ";":
            raise VerificationError("Preview MIME declarations are invalid")
        if "Actions" in entry:
            raise VerificationError("Preview action inventory is invalid")
    elif identity == "org.rmac.Player":
        if entry.get("MimeType") != ";".join(PLAYER_MIME_TYPES) + ";":
            raise VerificationError("Media Player MIME declarations are invalid")
        if "Actions" in entry:
            raise VerificationError("Media Player action inventory is invalid")
    elif identity == "org.rmac.ArchiveUtility":
        if entry.get("MimeType") != ";".join(ARCHIVE_MIME_TYPES) + ";":
            raise VerificationError("Archive Utility MIME declarations are invalid")
        if "Actions" in entry:
            raise VerificationError("Archive Utility action inventory is invalid")
    elif identity == "org.rmac.Files":
        if entry.get("MimeType") != "inode/directory;":
            raise VerificationError("Files MIME declarations are invalid")
        if "Actions" in entry:
            raise VerificationError("Files action inventory is invalid")
    elif any(key in entry for key in ("MimeType", "Actions")) or "%" in entry["Exec"]:
        raise VerificationError(f"desktop entry claims unsupported activation: {identity}")


def _verify_metainfo(root: Path, identity: str, specification: dict[str, object]) -> None:
    path = root / f"usr/share/metainfo/{identity}.metainfo.xml"
    raw, _mode = _regular_bytes(path, MAX_METADATA_BYTES)
    if b"<!DOCTYPE" in raw or b"<!ENTITY" in raw:
        raise VerificationError(f"AppStream metadata has external markup: {identity}")
    try:
        component = ET.fromstring(raw)
    except ET.ParseError as error:
        raise VerificationError(f"AppStream metadata is invalid: {identity}") from error
    if component.tag != "component" or component.attrib != {
        "type": "desktop-application"
    }:
        raise VerificationError(f"AppStream component type is invalid: {identity}")
    expected_scalars = {
        "id": identity,
        "metadata_license": "MIT",
        "project_license": "MIT",
        "name": specification["name"],
        "summary": specification["summary"],
    }
    for tag, expected in expected_scalars.items():
        if _localized(component, tag, None) != expected:
            raise VerificationError(f"AppStream field is invalid: {identity} {tag}")
    if _localized(component, "name", "hi") != HINDI[str(specification["name"])]:
        raise VerificationError(f"AppStream Hindi name is invalid: {identity}")
    if _localized(component, "summary", "hi") != HINDI[str(specification["summary"])]:
        raise VerificationError(f"AppStream Hindi summary is invalid: {identity}")
    paragraphs = component.findall("description/p")
    if len(paragraphs) < 2 or any(
        paragraph.text is None or len(paragraph.text.strip()) < 60
        for paragraph in paragraphs
    ):
        raise VerificationError(f"AppStream description is incomplete: {identity}")
    launchable = component.find("launchable")
    icon = component.find("icon")
    release = component.find("releases/release")
    rating = component.find("content_rating")
    developer = component.find("developer")
    if (
        launchable is None
        or launchable.attrib != {"type": "desktop-id"}
        or launchable.text != f"{identity}.desktop"
        or icon is None
        or icon.attrib != {"type": "stock"}
        or icon.text != identity
        or component.findtext("provides/binary") != specification["binary"]
        or component.findtext("url[@type='homepage']")
        != "https://github.com/snehacodex/rmac"
        or developer is None
        or developer.attrib != {"id": "org.rmac"}
        or developer.findtext("name") != "Lulo OS contributors"
        or release is None
        or release.attrib != {"version": "0.9.0-beta.1", "date": "2026-09-24"}
        or rating is None
        or rating.attrib != {"type": "oars-1.1"}
    ):
        raise VerificationError(f"AppStream integration fields are invalid: {identity}")


def _po_entries(raw: bytes, *, translated: bool) -> dict[str, str]:
    try:
        text = raw.decode("utf-8")
    except UnicodeDecodeError as error:
        raise VerificationError("translation catalog is not UTF-8") from error
    if translated and ("#, fuzzy" in text or 'msgstr ""\n\nmsgid' in text):
        raise VerificationError("translation catalog is fuzzy or incomplete")
    pairs = re.findall(r'^msgid "([^"]+)"\nmsgstr "([^"]*)"$', text, re.MULTILINE)
    entries = dict(pairs)
    if len(entries) != len(pairs):
        raise VerificationError("translation catalog has duplicate messages")
    if translated and any(not value for value in entries.values()):
        raise VerificationError("translation catalog has an untranslated message")
    return entries


def _verify_localization(root: Path) -> None:
    directory = root / "usr/share/doc/rmac-apps/localization"
    linguas, _ = _regular_bytes(directory / "LINGUAS", MAX_METADATA_BYTES)
    if linguas != b"hi\n":
        raise VerificationError("localization language inventory is invalid")
    expected_sources = {
        f"applications/{identity}.desktop" for identity in APPLICATIONS
    } | {f"metainfo/{identity}.metainfo.xml" for identity in APPLICATIONS}
    potfiles, _ = _regular_bytes(directory / "POTFILES.in", MAX_METADATA_BYTES)
    try:
        actual_sources = set(potfiles.decode("utf-8").splitlines())
    except UnicodeDecodeError as error:
        raise VerificationError("localization source inventory is not UTF-8") from error
    if actual_sources != expected_sources or len(actual_sources) != len(
        potfiles.decode("utf-8").splitlines()
    ):
        raise VerificationError("localization source inventory is not exact")
    template, _ = _regular_bytes(directory / "rmac-apps.pot", MAX_METADATA_BYTES)
    catalog, _ = _regular_bytes(directory / "hi.po", MAX_METADATA_BYTES)
    if set(_po_entries(template, translated=False)) != set(HINDI):
        raise VerificationError("translation template message inventory is not exact")
    if _po_entries(catalog, translated=True) != HINDI:
        raise VerificationError("Hindi translation catalog is stale")


def _verify_mimeapps(root: Path) -> None:
    raw, _mode = _regular_bytes(root / MIMEAPPS, MAX_METADATA_BYTES)
    parser = configparser.ConfigParser(interpolation=None, strict=True)
    parser.optionxform = str
    try:
        parser.read_string(raw.decode("utf-8"))
    except (UnicodeDecodeError, configparser.Error) as error:
        raise VerificationError("rmac MIME defaults are invalid") from error
    expected = {mime: "org.rmac.Preview.desktop" for mime in PREVIEW_MIME_TYPES}
    expected.update({mime: "org.rmac.TextEditor.desktop" for mime in TEXT_MIME_TYPES})
    expected.update({mime: "org.rmac.ArchiveUtility.desktop" for mime in ARCHIVE_MIME_TYPES})
    expected.update({mime: "org.rmac.Player.desktop" for mime in PLAYER_MIME_TYPES})
    expected.update({mime: "org.rmac.Preview.desktop" for mime in PREVIEW_MIME_TYPES})
    expected.update({mime: "org.rmac.TextEditor.desktop" for mime in TEXT_MIME_TYPES})
    expected["inode/directory"] = "org.rmac.Files.desktop"
    if parser.sections() != ["Default Applications"] or dict(
        parser["Default Applications"]
    ) != expected:
        raise VerificationError("rmac MIME defaults are not exact")


def _verify_superseded_apps(root: Path) -> None:
    raw, _mode = _regular_bytes(root / SUPERSEDED_APPS, MAX_METADATA_BYTES)
    try:
        text = raw.decode("utf-8")
    except UnicodeDecodeError as error:
        raise VerificationError("superseded-app list is not UTF-8") from error
    entries: dict[str, str] = {}
    for line in text.splitlines():
        stripped = line.strip()
        if not stripped or stripped.startswith("#"):
            continue
        if "=" not in stripped:
            raise VerificationError("superseded-app list has an unparsable line")
        superseded, _, equivalent = stripped.partition("=")
        superseded = superseded.strip()
        equivalent = equivalent.strip()
        if not superseded.endswith(".desktop") or not equivalent:
            raise VerificationError("superseded-app list entry is invalid")
        if superseded in entries:
            raise VerificationError("superseded-app list has a duplicate entry")
        entries[superseded] = equivalent
    if entries != SUPERSEDED_APPLICATIONS:
        raise VerificationError("superseded-app list does not match the shipped default")
    for equivalent in entries.values():
        if equivalent not in APPLICATIONS:
            raise VerificationError(
                f"superseded-app list names an app rmac does not ship: {equivalent}"
            )


def _verify_metadata(root: Path) -> None:
    for identity, specification in APPLICATIONS.items():
        _verify_desktop(root, identity, specification)
        _verify_metainfo(root, identity, specification)
    _verify_localization(root)
    _verify_mimeapps(root)
    _verify_superseded_apps(root)
    license_text, _ = _regular_bytes(
        root / "usr/share/doc/rmac-apps/copyright", MAX_METADATA_BYTES
    )
    inventory, _ = _regular_bytes(
        root / "usr/share/doc/rmac-apps/LICENSES.md", MAX_METADATA_BYTES
    )
    if b"MIT License" not in license_text or b"application icons: MIT" not in inventory:
        raise VerificationError("application package license inventory is incomplete")


def verify_tree(root: Path, *, exact_tree: bool = True) -> None:
    if not root.is_absolute() or root.is_symlink() or not root.is_dir():
        raise VerificationError("verification root must be an absolute ordinary directory")
    entries = _load_manifest(root)
    claimed: set[Path] = set()
    for entry in entries:
        relative = _safe_manifest_path(entry.get("path"))
        claimed.add(relative)
        expected_mode = entry.get("mode")
        expected_hash = entry.get("sha256")
        if (
            not isinstance(expected_mode, str)
            # Only plain data and executables: never setuid, setgid,
            # sticky or world-writable, even if the tree matches.
            or expected_mode not in {"0644", "0755"}
            or not isinstance(expected_hash, str)
            or not re.fullmatch(r"[0-9a-f]{64}", expected_hash)
        ):
            raise VerificationError("application package manifest metadata is invalid")
        contents, mode = _regular_bytes(root / relative)
        if mode != int(expected_mode, 8):
            raise VerificationError(f"installed mode differs: {relative.name}")
        if hashlib.sha256(contents).hexdigest() != expected_hash:
            raise VerificationError(f"installed content differs: {relative.name}")
    if claimed != EXPECTED_PATHS:
        raise VerificationError("application package manifest inventory is not exact")
    if exact_tree:
        actual = _tree_entries(root)
        expected = _with_parent_directories(claimed | {MANIFEST})
        if actual != expected:
            raise VerificationError("application package tree contains an unexpected path")
    _verify_metadata(root)


def _run_validator(command: list[str], label: str) -> None:
    try:
        result = subprocess.run(
            command,
            check=False,
            capture_output=True,
            timeout=30,
        )
    except (OSError, subprocess.TimeoutExpired) as error:
        raise VerificationError(f"{label} could not run") from error
    if result.returncode != 0:
        raise VerificationError(f"{label} rejected rmac metadata")


def _run_standard_validators(
    root: Path, desktop_validator: Path, appstream_validator: Path
) -> None:
    desktop_paths = [
        str(root / f"usr/share/applications/{identity}.desktop")
        for identity in APPLICATIONS
    ]
    _run_validator(
        [str(desktop_validator), *desktop_paths],
        "desktop-file-validate",
    )
    for identity in APPLICATIONS:
        _run_validator(
            [
                str(appstream_validator),
                "validate",
                "--no-net",
                str(root / f"usr/share/metainfo/{identity}.metainfo.xml"),
            ],
            "appstreamcli",
        )


def _path_validator(name: str) -> Path:
    candidate = shutil.which(name)
    if candidate is None:
        raise VerificationError(f"required metadata validator is missing: {name}")
    path = Path(candidate)
    if not path.is_file() or not os.access(path, os.X_OK):
        raise VerificationError(f"required metadata validator is missing: {name}")
    return path


def verify_standard_metadata(root: Path) -> None:
    """Verify one exact staged tree with the host freedesktop validators."""
    verify_tree(root)
    _run_standard_validators(
        root,
        _path_validator("desktop-file-validate"),
        _path_validator("appstreamcli"),
    )


def verify_installed_host(root: Path) -> None:
    verify_tree(root, exact_tree=False)
    for specification in APPLICATIONS.values():
        executable = root / "usr/bin" / str(specification["binary"])
        if not executable.is_file() or not os.access(executable, os.X_OK):
            raise VerificationError(f"required application is missing: {executable.name}")
    desktop_validator = root / "usr/bin/desktop-file-validate"
    appstream_validator = root / "usr/bin/appstreamcli"
    for validator in (desktop_validator, appstream_validator):
        if not validator.is_file() or not os.access(validator, os.X_OK):
            raise VerificationError(f"required metadata validator is missing: {validator.name}")
    _run_standard_validators(root, desktop_validator, appstream_validator)


def main() -> int:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--root", required=True, type=Path)
    mode = parser.add_mutually_exclusive_group()
    mode.add_argument(
        "--installed-host",
        action="store_true",
        help="also require app binaries and run freedesktop metadata validators",
    )
    mode.add_argument(
        "--standard-validators",
        action="store_true",
        help="run host freedesktop validators against an exact staged tree",
    )
    arguments = parser.parse_args()
    try:
        if arguments.installed_host:
            verify_installed_host(arguments.root)
        elif arguments.standard_validators:
            verify_standard_metadata(arguments.root)
        else:
            verify_tree(arguments.root)
    except VerificationError as error:
        parser.exit(4, f"verify-application-package: {error}\n")
    print("rmac application metadata verified")
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
