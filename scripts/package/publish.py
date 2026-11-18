#!/usr/bin/env python3
# /// script
# requires-python = ">=3.11"
# dependencies = ["jsonschema>=4.23,<5"]
# ///
"""Publish Superfile and synchronize its stable Hyperscoop manifest."""
import argparse
from concurrent.futures import ThreadPoolExecutor
import hashlib
import json
import os
from pathlib import Path
import re
import shutil
import struct
import subprocess
import sys
import tempfile
import time
import tomllib
from urllib.error import HTTPError, URLError
from urllib.parse import urlencode
from urllib.request import Request, urlopen
import zipfile

import release

OWNER = "Super1Windcloud"
REPOSITORY = f"{OWNER}/SuperfileView"
RELEASES = f"https://github.com/{REPOSITORY}/releases"
STABLE_MANIFEST = f"{RELEASES}/latest/download/latest.json"
SCOOP_SCHEMA = "https://raw.githubusercontent.com/ScoopInstaller/Scoop/master/schema.json"
SUBMODULE = "hyperscoop_source_bucket"
MANIFEST = "bucket/superfile.json"
VERSION_FILES = ("Cargo.toml", "Cargo.lock")


def require(condition, message):
    if not condition:
        raise ValueError(message)


def say(message):
    print(message, flush=True)


class Repository:
    def __init__(self, path, name, branch):
        self.path = path.resolve()
        self.ref = f"refs/heads/{branch}"
        require(Path(self.git("rev-parse", "--show-toplevel")).resolve() == self.path,
                f"Expected a checkout at {self.path}")
        local_branch = self.git("symbolic-ref", "--short", "HEAD")
        require(local_branch == branch, f"Check out {branch} in {self.path}")
        self.remote = self.git("config", "--get", f"branch.{local_branch}.remote")
        require(self.git("config", "--get", f"branch.{local_branch}.merge") == self.ref,
                f"{self.path} must track {self.ref}")
        expected = f"{OWNER}/{name}".lower()
        for flags in [(), ("--push",)]:
            urls = self.git("remote", "get-url", "--all", *flags, self.remote).splitlines()
            require(len(urls) == 1 and re.fullmatch(
                rf"(?:https://github\.com/|git@github\.com:|ssh://git@github\.com/){re.escape(expected)}(?:\.git)?",
                urls[0], flags=re.IGNORECASE,
            ), f"{self.path} must use GitHub repository {expected}")

    def git(self, *args, capture=True):
        result = subprocess.run(
            ["git", *args], cwd=self.path, check=True, stdout=subprocess.PIPE if capture else None,
            encoding="utf-8" if capture else None,
        )
        return result.stdout.rstrip("\r\n") if capture else None

    def remote_refs(self, *patterns):
        return {
            ref: oid for oid, ref in (
                line.split() for line in self.git("ls-remote", self.remote, *patterns).splitlines()
            )
        }

    def fetch_tip(self):
        self.git("fetch", "--no-tags", self.remote, self.ref, capture=False)
        return self.git("rev-parse", "FETCH_HEAD")

    def clean(self, *paths):
        return not self.git("status", "--porcelain", "--untracked-files=all", "--", *paths)

    def push(self, *refs):
        self.git("push", "--atomic", self.remote, *refs, capture=False)


def stable_version(tag):
    parsed = release.version(tag)
    require(parsed["prerelease"] == "false", "Use a stable version such as v1.2.3")
    return tuple(map(int, parsed["core"].split(".")))


def latest_stable_tag(refs):
    versions = []
    for ref in refs:
        name = ref.removeprefix("refs/tags/")
        if re.fullmatch(r"v?\d+\.\d+\.\d+", name):
            versions.append(stable_version(name))
    return max(versions, default=(0, 0, 0))


def proposed_tag(requested, workspace, remote_latest):
    if requested in ("major", "minor", "patch"):
        parts = list(max(stable_version(workspace), remote_latest))
        index = ("major", "minor", "patch").index(requested)
        parts[index] += 1
        parts[index + 1:] = [0] * (2 - index)
        requested = ".".join(map(str, parts))
    value = stable_version(requested)
    require(value >= stable_version(workspace) and value > remote_latest,
            "The release must advance the remote tags and cannot downgrade the workspace")
    return "v" + ".".join(map(str, value))


def manifest_version(root):
    return tomllib.loads((root / "Cargo.toml").read_text(encoding="utf-8"))["workspace"]["package"]["version"]


def gitlink(parent, ref):
    entry = parent.git("ls-tree", ref, "--", SUBMODULE).split()
    require(len(entry) == 4 and entry[:2] == ["160000", "commit"], "Hyperscoop bucket must be a submodule")
    return entry[2]


def check_bucket(parent, bucket, resume=False):
    require(not parent.git("diff", "--cached", "--name-only", "--", SUBMODULE),
            f"Unstage the existing {SUBMODULE} change before publishing")
    parent_head, bucket_head = parent.git("rev-parse", "HEAD"), bucket.git("rev-parse", "HEAD")
    gitlink(parent, parent_head)
    if not resume:
        require(bucket.clean(MANIFEST), f"Commit or stash changes to {bucket.path / MANIFEST} first")
        require(gitlink(parent, parent_head) == bucket_head, "The Hyperscoop submodule checkout differs from its recorded pointer")
        for repo, head in [(parent, parent_head), (bucket, bucket_head)]:
            require(repo.remote_refs(repo.ref).get(repo.ref) == head, f"Synchronize {repo.path} with its upstream first")


def run_checks(root):
    for command in [
        ["cargo", "fmt", "--all", "--check"],
        ["cargo", "clippy", "--workspace", "--all-targets", "--locked", "--", "-D", "warnings"],
        ["cargo", "test", "--workspace", "--locked"],
        [sys.executable, "scripts/check-locales.py"],
        [sys.executable, "scripts/check-architecture.py"],
        [sys.executable, "-m", "unittest", "discover", "-s", "scripts/package", "-p", "test_*.py"],
    ]:
        say(f"Running {' '.join(command)}")
        subprocess.run(command, cwd=root, check=True)


def publish_tag(source, tag):
    remote_tip = source.fetch_tip()
    source.git("merge-base", "--is-ancestor", remote_tip, "HEAD")
    local_tag = source.git("tag", "--list", tag)
    require(not local_tag, f"Tag {tag} already exists locally. Use just publish-resume {tag}.")
    say(f"Validating {tag} before pushing")
    run_checks(source.path)
    require(source.clean(), "The worktree changed during release validation")
    if manifest_version(source.path) != tag[1:]:
        release.prepare(tag, source.path)
        subprocess.run(["cargo", "metadata", "--locked", "--no-deps", "--format-version", "1"],
                       cwd=source.path, check=True, stdout=subprocess.DEVNULL)
    source.git("diff", "--check", "HEAD")
    changed = set(source.git("status", "--porcelain", "--untracked-files=all").splitlines())
    require(all(item[3:] in VERSION_FILES for item in changed),
            "Unexpected workspace changes appeared during validation")
    if changed:
        source.git("commit", "--only", "-m", f"chore(release): prepare {tag}", "--", *VERSION_FILES,
                   capture=False)
    source.git("tag", "-a", tag, "-m", f"Superfile {tag[1:]}")
    push_tag(source, tag)


def push_tag(source, tag):
    require(source.clean(), "Commit the Superfile worktree before pushing a release")
    ref = f"refs/tags/{tag}"
    require(source.git("rev-parse", f"{ref}^{{commit}}") == source.git("rev-parse", "HEAD")
            and manifest_version(source.path) == tag[1:],
            "The local tag must point to HEAD at the requested workspace version")
    source.push(f"HEAD:{source.ref}", ref)


def remote_tag_commit(source, tag):
    ref = f"refs/tags/{tag}"
    refs = source.remote_refs(ref, f"{ref}^{{}}")
    return refs.get(f"{ref}^{{}}", refs.get(ref))


def request(url, deadline):
    headers = {"User-Agent": "Superfile-publisher", "Cache-Control": "no-cache"}
    api = url.startswith("https://api.github.com/")
    if api:
        headers["Accept"] = "application/vnd.github+json"
        headers["X-GitHub-Api-Version"] = "2022-11-28"
    req = Request(url, headers=headers)
    if api and (token := os.environ.get("GH_TOKEN") or os.environ.get("GITHUB_TOKEN")):
        req.add_unredirected_header("Authorization", f"Bearer {token}")
    while True:
        try:
            return urlopen(req, timeout=60)
        except HTTPError as error:
            retry = error.headers.get("Retry-After")
            reset = error.headers.get("X-RateLimit-Reset")
            limited = api and error.headers.get("X-RateLimit-Remaining") == "0"
            if limited and reset and reset.isdigit():
                delay = max(15, int(reset) - time.time() + 2)
            elif error.code in (429, 500, 502, 503, 504) or error.code == 403 and retry:
                delay = int(retry) if retry and retry.isdigit() else 30
            else:
                raise
            error.close()
        except URLError:
            delay = 30
        require(time.monotonic() + delay < deadline, f"Timed out requesting {url}")
        say(f"Request temporarily unavailable, retrying in {delay:.0f}s: {url}")
        time.sleep(delay)


def read_json(url, deadline):
    with request(url, deadline) as response:
        return json.load(response)


def wait_for_actions(tag, commit, deadline):
    params = urlencode({"event": "push", "branch": tag, "head_sha": commit, "per_page": 10})
    url = f"https://api.github.com/repos/{REPOSITORY}/actions/workflows/build.yml/runs?{params}"
    last = None
    while time.monotonic() < deadline:
        runs = read_json(url, deadline)["workflow_runs"]
        run = next((item for item in runs if item["head_sha"] == commit and item["head_branch"] == tag), None)
        status = run["status"] if run else "waiting for the tag workflow"
        if status != last:
            say(f"{tag}: {status}" + (f" ({run['html_url']})" if run else ""))
            last = status
        if run and status == "completed":
            require(run["conclusion"] == "success", f"Release workflow {run['conclusion']}: {run['html_url']}")
            return run["html_url"]
        time.sleep(min(60, max(0, deadline - time.monotonic())))
    raise ValueError(f"Timed out waiting for the {tag} workflow")


def sha256(path):
    with path.open("rb") as stream:
        return hashlib.file_digest(stream, "sha256").hexdigest()


def check_stable_manifest(manifest, deadline):
    tag = json.loads(manifest)["tag"]
    with request(f"{STABLE_MANIFEST}?{urlencode({'tag': tag})}", deadline) as response:
        require(response.read() == manifest, "The latest stable manifest does not match this release")


def verify_release(tag, directory, deadline):
    metadata = read_json(f"https://api.github.com/repos/{REPOSITORY}/releases/tags/{tag}", deadline)
    require(metadata["tag_name"] == tag and not metadata["draft"] and not metadata["prerelease"],
            f"{tag} must be a published stable release")
    packages = release.assets(tag)
    expected = packages | {"latest.json", "SHA256SUMS.txt"}
    assets = metadata["assets"]
    require(len(assets) == len(expected) and {item["name"] for item in assets} == expected,
            "GitHub Release does not have all eight expected assets")

    def download(asset):
        name = asset["name"]
        path = directory / name
        url = f"{RELEASES}/download/{tag}/{name}"
        require(asset["browser_download_url"] == url and asset["state"] == "uploaded" and asset["size"] > 0,
                f"Invalid uploaded asset: {name}")
        with request(url, deadline) as response, path.open("wb") as destination:
            shutil.copyfileobj(response, destination)
        digest = sha256(path)
        require(path.stat().st_size == asset["size"], f"Size mismatch: {name}")
        if asset.get("digest"):
            require(asset["digest"] == f"sha256:{digest}", f"GitHub asset digest mismatch: {name}")
        say(f"Verified {name}")
        return name, digest

    with ThreadPoolExecutor(max_workers=4) as pool:
        hashes = dict(pool.map(download, assets))
    checksums = (directory / "SHA256SUMS.txt").read_text(encoding="utf-8")
    expected_lines = {f"{hashes[name]}  {name}" for name in packages}
    require(len(checksums.splitlines()) == len(packages) and set(checksums.splitlines()) == expected_lines,
            "Downloaded asset hashes do not match SHA256SUMS.txt")
    release.verify_manifest(tag, directory, REPOSITORY)
    manifest = (directory / "latest.json").read_bytes()
    check_stable_manifest(manifest, deadline)
    return manifest, checksums, hashes


def validate_windows_zip(path, manifest):
    with zipfile.ZipFile(path) as archive:
        require(archive.testzip() is None, "Windows ZIP failed its integrity check")
        require(manifest["bin"] == "superfile.exe" and all(
            archive.getinfo(shortcut[0]).file_size > 0 for shortcut in manifest["shortcuts"]
        ), "Scoop executable or shortcut targets are invalid")
        with archive.open(manifest["bin"]) as executable:
            dos = executable.read(64)
            require(len(dos) == 64 and dos.startswith(b"MZ"), "Windows executable has no DOS header")
            executable.seek(struct.unpack_from("<I", dos, 60)[0])
            pe = executable.read(26)
            require(len(pe) == 26 and pe[:4] == b"PE\0\0"
                    and struct.unpack_from("<H", pe, 4)[0] == 0x8664
                    and struct.unpack_from("<H", pe, 24)[0] == 0x20B, "Expected an x64 PE32+ executable")


def bucket_manifest(current, tag, directory, schema, checksums, hashes):
    import jsonschema

    document = json.loads(current)
    version = tag[1:]
    require(stable_version(document["version"]) <= stable_version(version), "The bucket already has a newer version")
    name = f"superfile-{tag}-windows-x86_64.zip"
    url = f"{RELEASES}/download/{tag}/{name}"
    digest = hashes[name]
    document["version"] = version
    document["architecture"]["64bit"].update(url=url, hash=digest)
    try:
        jsonschema.validate(document, schema, format_checker=jsonschema.FormatChecker())
    except jsonschema.ValidationError as error:
        raise ValueError(f"Invalid Scoop manifest: {error.message}") from error
    require(document["checkver"] == {"url": STABLE_MANIFEST, "jsonpath": "$.version"},
            "Scoop checkver does not read the stable manifest version")
    update = document["autoupdate"]
    require(update["architecture"]["64bit"]["url"].replace("$version", version) == url
            and update["hash"]["url"].replace("$version", version) == f"{RELEASES}/download/{tag}/SHA256SUMS.txt",
            "Scoop autoupdate does not resolve to the verified ZIP and checksum list")
    pattern = update["hash"]["regex"].replace("$sha256", "([a-fA-F0-9]{64})").replace("$basename", re.escape(name))
    require(re.findall(pattern, checksums) == [digest], "Scoop checksum rule did not find the verified hash")
    validate_windows_zip(directory / name, document)
    return document


def pending_bucket_change(repo, base, path, message, is_expected):
    head = repo.git("rev-parse", "HEAD")
    if head != base:
        require(repo.git("rev-list", "--parents", "-n", "1", head).split() == [head, base]
                and repo.git("diff", "--name-only", base, head) == path
                and repo.git("log", "-1", "--format=%s", head) == message
                and is_expected(head), f"Unrelated commits in {repo.path}. Synchronize it with its upstream.")
    return head


def push_bucket_change(repo, base, path, message, is_expected):
    head = pending_bucket_change(repo, base, path, message, is_expected)
    if head == base and not is_expected(head):
        repo.git("commit", "--only", "-m", message, "--", path, capture=False)
        head = repo.git("rev-parse", "HEAD")
        require(is_expected(head), f"Unexpected staged release content in {repo.path}")
    if head != base:
        repo.push(f"HEAD:{repo.ref}")
    require(repo.fetch_tip() == head and is_expected(head), f"Could not verify {path} on the upstream of {repo.path}")
    return head


def sync_bucket(parent, bucket, tag, directory, verified, deadline):
    manifest, checksums, hashes = verified
    check_bucket(parent, bucket, resume=True)
    path = bucket.path / MANIFEST
    current = path.read_bytes()
    tracked = bucket.git("show", f"HEAD:{MANIFEST}").encode()
    desired = bucket_manifest(tracked, tag, directory, read_json(SCOOP_SCHEMA, deadline), checksums, hashes)
    require(bucket.clean(MANIFEST) or (
        json.loads(current) == desired
        and json.loads(bucket.git("show", f":{MANIFEST}")) in (json.loads(tracked), desired)
    ), f"Unrelated changes in {path}")
    version = tag[1:]
    bucket_message = f"chore(bucket): update Superfile to {version}"
    parent_message = f"chore(bucket): sync Superfile {version} manifest"

    def matches_manifest(ref):
        return json.loads(bucket.git("show", f"{ref}:{MANIFEST}")) == desired

    bucket_base = bucket.fetch_tip()
    parent_base = parent.fetch_tip()
    bucket_tip = pending_bucket_change(bucket, bucket_base, MANIFEST, bucket_message, matches_manifest)
    pending_bucket_change(
        parent, parent_base, SUBMODULE, parent_message,
        lambda ref: gitlink(parent, ref) == bucket_tip and matches_manifest(bucket_tip),
    )
    check_stable_manifest(manifest, deadline)
    if json.loads(current) != desired:
        newline = "\r\n" if b"\r\n" in current else "\n"
        updated = (json.dumps(desired, ensure_ascii=False, indent=4) + "\n").replace("\n", newline)
        with tempfile.NamedTemporaryFile(dir=path.parent, delete=False) as output:
            output.write(updated.encode())
            temporary = Path(output.name)
        try:
            temporary.chmod(path.stat().st_mode)
            temporary.replace(path)
        finally:
            temporary.unlink(missing_ok=True)
    bucket_commit = push_bucket_change(bucket, bucket_base, MANIFEST, bucket_message, matches_manifest)
    parent_commit = push_bucket_change(
        parent, parent_base, SUBMODULE, parent_message, lambda ref: gitlink(parent, ref) == bucket_commit,
    )
    say(f"Verified Hyperscoop bucket {bucket_commit} and parent pointer {parent_commit}")


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("command", choices=["publish", "plan", "resume"])
    parser.add_argument("version", nargs="?", default="patch")
    parser.add_argument("--hyperscoop", type=Path,
                        default=Path(os.environ.get("SUPERFILE_HYPERSCOOP", Path.home() / "demo/hyperscoop")))
    parser.add_argument("--timeout-minutes", type=int, default=240)
    args = parser.parse_args()
    tag = None
    try:
        require(args.timeout_minutes > 0, "The wait timeout must be positive")
        source = Repository(release.ROOT, "SuperfileView", "main")
        parent = Repository(args.hyperscoop, "hyperscoop", "main")
        bucket = Repository(args.hyperscoop / SUBMODULE, SUBMODULE, "master")
        check_bucket(parent, bucket, resume=args.command == "resume")
        if args.command == "resume":
            tag = "v" + ".".join(map(str, stable_version(args.version)))
            if not remote_tag_commit(source, tag):
                check_bucket(parent, bucket)
                push_tag(source, tag)
        else:
            remote_tags = source.remote_refs("refs/tags/*")
            tag = proposed_tag(args.version, manifest_version(source.path), latest_stable_tag(remote_tags))
            require(not source.git("tag", "--list", tag),
                    f"Tag {tag} already exists locally. Use just publish-resume {tag}.")
            say(f"Release {tag} from {source.git('rev-parse', '--short', 'HEAD')} and sync {bucket.path}")
            if args.command == "plan":
                if not source.clean():
                    say("Commit the current Superfile changes before publishing.")
                return
            require(source.clean(), "Commit Superfile changes before starting a release")
            publish_tag(source, tag)
        commit = remote_tag_commit(source, tag)
        require(commit, f"Could not verify the pushed {tag} tag")
        deadline = time.monotonic() + args.timeout_minutes * 60
        run_url = wait_for_actions(tag, commit, deadline)
        with tempfile.TemporaryDirectory(prefix=f"superfile-{tag}-") as scratch:
            directory = Path(scratch)
            verified = verify_release(tag, directory, deadline)
            sync_bucket(parent, bucket, tag, directory, verified, deadline)
        say(f"Published {RELEASES}/tag/{tag}\nActions: {run_url}")
    except (OSError, ValueError, KeyError, subprocess.CalledProcessError, zipfile.BadZipFile) as error:
        print(f"Release stopped: {error}", file=sys.stderr)
        if tag and args.command != "plan":
            print(f"For an existing tag, continue with: just publish-resume {tag}", file=sys.stderr)
        return 1


if __name__ == "__main__":
    sys.exit(main())
