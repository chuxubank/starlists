#!/usr/bin/env python3
"""Example classifier: rebuild a purpose-based Star List taxonomy.

This is a starting point, not a universal scheme. Export a corpus with
`stars export --for-agent`, run this (or your own) script, then
`stars propose import` the JSON it writes.
"""

from __future__ import annotations

import json
import re
from collections import Counter, defaultdict
from pathlib import Path

TARGET = [
    ("emacs", "Emacs", "Emacs, Org, and Elisp packages."),
    ("android", "Android", "Android apps, system, and device tooling."),
    ("kotlin", "Kotlin", "Kotlin language, KMP, Compose Multiplatform, Gradle."),
    ("apple", "Apple", "Swift, iOS, and Apple-platform development."),
    ("desktop", "Desktop", "End-user desktop/GUI apps for Mac, Windows, Linux."),
    ("cli", "CLI", "Terminal, shell, and command-line tools."),
    ("ai", "AI", "LLM apps, agents, MCP, and coding assistants."),
    ("ml", "ML", "Models, training, computer vision, and diffusion."),
    ("web", "Web", "Web apps, frontend, and browser tooling."),
    ("proxy", "Proxy", "Proxies, VPN, DNS, and traffic routing."),
    ("infra", "Infra", "CI, deploy, PaaS, databases, and automation."),
    ("home", "Home", "Home Assistant, IoT, MQTT, and hardware."),
    ("keyboard", "Keyboard", "Keyboards, layouts, ZMK/QMK."),
    ("writing", "Writing", "Fonts, TeX, Zotero, PDF, and typography."),
    ("finance", "Finance", "Ledger, trading, crypto, and money tools."),
    ("security", "Security", "Reverse engineering, ROM, and security tools."),
    ("learning", "Learning", "Courses, interviews, compilers, and textbooks."),
    ("media", "Media", "Audio, music, video, and creative graphics."),
    ("life", "Life", "Health, time tracking, cooking, and daily life."),
]

INHERIT = {
    "emacs": "emacs",
    "android": "android",
    "kotlin": "kotlin",
    "proxy": "proxy",
    "keyboard": "keyboard",
    "finance": "finance",
    "count": "finance",
    "web": "web",
    "cli": "cli",
    "editor": "cli",
    "ai": "ai",
    "swift": "apple",
    "cv-cg": "ml",
    "audio": "media",
    "course": "learning",
    "interview": "learning",
    "algorithm": "learning",
    "lang": "learning",
    "latex": "writing",
    "font": "writing",
    "infra": "infra",
    "paas": "infra",
    "database": "infra",
    "automation": "infra",
    "hass": "home",
    "hardware": "home",
    "hack": "security",
    "reverse": "security",
    "health": "life",
    "time": "life",
}

# (list, weight, patterns) — matched against repo/desc/topics/language
RULES: list[tuple[str, int, list[str]]] = [
    ("emacs", 8, [r"\bemacs\b", r"\belisp\b", r"\borg-mode\b", r"\.el$", r"emacs lisp"]),
    ("android", 6, [r"\bandroid\b", r"jetpack", r"shizuku", r"apk", r"adb\b"]),
    ("kotlin", 6, [r"\bkotlin\b", r"kmp\b", r"ktor\b", r"compose-multiplatform", r"jetbrains/"]),
    ("apple", 6, [r"\bswiftui\b", r"\bswift\b", r"\bios\b", r"uikit", r"xcode"]),
    ("proxy", 8, [r"v2ray", r"xray", r"sing-box", r"clash", r"shadowsocks", r"trojan", r"hysteria", r"dnscrypt", r"socks5", r"\bvpn\b"]),
    ("proxy", 5, [r"(?<!ai )(?<!llm )\bproxy\b"]),
    ("keyboard", 8, [r"\bzmk\b", r"\bqmk\b", r"keymap", r"colemak", r"keyboard-layout", r"silakka", r"crkbd", r"\bkeyboard\b"]),
    ("finance", 7, [r"beancount", r"plaintext-accounting", r"trading", r"\bbtc\b", r"bitcoin", r"ethereum", r"metamask", r"\bfinance\b", r"ledger", r"alipay"]),
    ("ai", 5, [r"\bmcp\b", r"ai-agent", r"langchain", r"llamaindex", r"ollama", r"litellm", r"openai", r"chatgpt", r"claude", r"aider", r"cursor", r"llm", r"gpt-"]),
    ("ml", 6, [r"computer-vision", r"object-detection", r"diffusion", r"comfyui", r"stable-diffusion", r"pytorch", r"tensorflow", r"colmap", r"whisper", r"onnx", r"lora\b", r"deep-learning"]),
    ("web", 5, [r"\breact\b", r"\bvue\b", r"\bnext\.?js\b", r"tailwind", r"\bhono\b", r"\bremix\b", r"typescript", r"frontend"]),
    ("cli", 4, [r"\bcli\b", r"\bterminal\b", r"\btui\b", r"\bshell\b", r"\bzsh\b", r"starship", r"neovim", r"helix-editor", r"\bfzf\b"]),
    ("infra", 5, [r"\bkubernetes\b", r"\bdocker\b", r"\bansible\b", r"\bterraform\b", r"github-actions", r"\bhelm\b", r"coolify", r"dokku", r"\bpostgres", r"\bdatabase\b", r"argo-cd"]),
    ("home", 7, [r"home-assistant", r"\bhacs\b", r"\bfrigate\b", r"go2rtc", r"\bmqtt\b", r"homeassistant", r"xiaomi", r"rtsp"]),
    ("writing", 6, [r"\bzotero\b", r"\blatex\b", r"\btex\b", r"\bfont\b", r"typeface", r"\bpdf\b", r"typst", r"katex"]),
    ("security", 6, [r"ghidra", r"radare", r"reverse-engineering", r"disassembler", r"\bfrida\b", r"unidbg", r"jailbreak"]),
    ("learning", 5, [r"interview", r"leetcode", r"textbook", r"\bcourse\b", r"rust-book", r"cs6120", r"sicp", r"awesome-"]),
    ("media", 5, [r"\baudio\b", r"\bmusic\b", r"\bmidi\b", r"audacity", r"musescore", r"ffmpeg", r"noise-suppression"]),
    ("life", 5, [r"activitywatch", r"wakatime", r"habitica", r"cookbook", r"fittrackee", r"quantified-self"]),
    ("desktop", 5, [r"menubar", r"status bar", r"look and feel", r"\bgtk\b", r"\bqt\b", r"electron app", r"\btauri\b", r"file manager", r"window manager"]),
    ("learning", 6, [r"anki", r"flashcard", r"sicp", r"课程", r"公开课", r"cheatsheet", r"cheat-sheet"]),
    ("media", 6, [r"handbrake", r"ffmpeg", r"gimp", r"blender", r"freecad", r"unreal", r"gamescope", r"proton", r"netease", r"niconico"]),
    ("web", 5, [r"leaflet", r"starlette", r"asgi", r"rsshub", r"freshrss"]),
    ("infra", 5, [r"dnscontrol", r"webdav", r"self-host", r"selfhost", r"\balist\b"]),
    ("security", 5, [r"owasp", r"cheat sheet series"]),
    ("writing", 5, [r"grammar", r"harper", r"mupdf", r"pdfium"]),
    ("apple", 4, [r"itlwm", r"sidestore", r"ipa\b"]),
    ("ml", 4, [r"\bmanim\b", r"3d-reconstruction", r"gaussian", r"nerf"]),
]

# repo-name / org overrides (high confidence)
NAME_HINTS: list[tuple[str, str]] = [
    ("beancount", "finance"),
    ("v2ray", "proxy"),
    ("xray", "proxy"),
    ("sing-box", "proxy"),
    ("clash", "proxy"),
    ("shadowrocket", "proxy"),
    ("zmk", "keyboard"),
    ("qmk", "keyboard"),
    ("emacs", "emacs"),
    ("org-mode", "emacs"),
    ("home-assistant", "home"),
    ("frigate", "home"),
    ("zotero", "writing"),
]


def blob(repo: dict) -> str:
    parts = [
        repo.get("repo") or "",
        repo.get("description") or "",
        repo.get("language") or "",
        " ".join(repo.get("topics") or []),
    ]
    return " ".join(parts).lower()


def score_repo(repo: dict) -> dict[str, int]:
    text = blob(repo)
    scores: dict[str, int] = defaultdict(int)
    name = (repo.get("repo") or "").lower()
    lang = (repo.get("language") or "").lower()
    topics = {t.lower() for t in (repo.get("topics") or [])}

    for needle, dest in NAME_HINTS:
        if needle in name or needle in text:
            scores[dest] += 10

    for dest, weight, pats in RULES:
        for pat in pats:
            if re.search(pat, text):
                scores[dest] += weight
                break

    for old in repo.get("lists") or []:
        if dest := INHERIT.get(old):
            bonus = 2 if dest in {"security", "home"} else 4
            scores[dest] += bonus

    if lang == "emacs lisp":
        scores["emacs"] += 12
    if lang == "kotlin" and "android" in topics:
        scores["android"] += 3
        scores["kotlin"] += 4
    if lang == "swift":
        scores["apple"] += 5
    if lang == "java" and ("android" in topics or "android" in text):
        scores["android"] += 6

    # desktop vs apple: SwiftUI menubar utilities are desktop+apple
    if scores["desktop"] and scores["apple"] and "swift" in text:
        scores["apple"] += 1

    return dict(scores)


def assign(repo: dict) -> list[str]:
    scores = score_repo(repo)
    if not scores:
        lang = (repo.get("language") or "").lower()
        text = blob(repo)
        if lang == "emacs lisp":
            return ["emacs"]
        if lang == "swift":
            return ["apple"]
        if lang == "kotlin":
            return ["kotlin"]
        if "android" in text:
            return ["android"]
        if any(k in text for k in ("course", "book", "tutorial", "guide", "awesome")):
            return ["learning"]
        if any(k in text for k in ("app", "gui", "macos", "windows", "desktop")):
            return ["desktop"]
        return ["cli"]

    ranked = sorted(scores.items(), key=lambda kv: (-kv[1], kv[0]))
    chosen = [ranked[0][0]]
    # second list only if close and distinct purpose
    if len(ranked) > 1 and ranked[1][1] >= max(6, ranked[0][1] * 0.75):
        if ranked[1][0] != ranked[0][0]:
            chosen.append(ranked[1][0])
    # cap at 2 for even-ish partitions
    return chosen[:2]


def main() -> None:
    corpus = json.loads(Path("/tmp/stars-corpus.json").read_text())
    repos = corpus["repos"]
    memberships = []
    by = defaultdict(list)
    for repo in repos:
        lists = assign(repo)
        memberships.append({"repo": repo["repo"], "lists": lists})
        for sl in lists:
            by[sl].append(repo)

    print("distribution")
    for slug, _name, _ in TARGET:
        items = by[slug]
        print(f"  {slug:12} {len(items):4}")
    unused = [slug for slug, _, _ in TARGET if slug not in by]
    if unused:
        print("empty", unused)
    print("multi", sum(1 for m in memberships if len(m["lists"]) > 1))
    print("total memberships", sum(len(m["lists"]) for m in memberships))

    keep_slugs = {t[0] for t in TARGET}
    existing = {x["slug"] for x in corpus["lists"]}
    creates = [
        {
            "slug": slug,
            "name": name,
            "description": desc,
            "is_private": False,
        }
        for slug, name, desc in TARGET
        if slug not in existing
    ]
    deletes = sorted(existing - keep_slugs)
    # rename none; we create new slugs and delete old

    proposal = {
        "version": 1,
        "id": "PLAN_RETAX_20260818",
        "notes": (
            "Rebuild Star Lists: dissolve OSS/python and tiny lists; "
            "purpose-based taxonomy; multi-list allowed (usually 1-2)."
        ),
        "lists": {
            "create": creates,
            "rename": [],
            "update": [
                {"slug": slug, "description": desc}
                for slug, _name, desc in TARGET
                if slug in existing
            ],
            "merge": [],
            "delete": deletes,
        },
        "memberships": memberships,
    }
    out = Path("/tmp/stars-proposal-retax.json")
    out.write_text(json.dumps(proposal, ensure_ascii=False, indent=2) + "\n")
    print("wrote", out, "creates", len(creates), "deletes", len(deletes), "memberships", len(memberships))

    # dump samples for review
    review = Path("/tmp/stars-retax-samples.txt")
    with review.open("w") as f:
        for slug, name, _ in TARGET:
            f.write(f"\n## {slug} ({name}) n={len(by[slug])}\n")
            for r in by[slug][:12]:
                f.write(f"  - {r['repo']}: {(r.get('description') or '')[:90]}\n")
    print("samples", review)


if __name__ == "__main__":
    main()
