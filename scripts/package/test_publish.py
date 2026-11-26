import io
import json
from pathlib import Path
import struct
import subprocess
import tempfile
import time
import unittest
from unittest.mock import patch
import zipfile

import publish
import release


class LocalRepository(publish.Repository):
    """Use a disposable Git remote while exercising the production Git commands."""

    def __init__(self, path, remote):
        self.path = path
        self.remote = "origin"
        self.ref = "refs/heads/main"
        subprocess.run(["git", "init", "--bare", "-q", str(remote)], check=True)
        self.git("init", "-q", "-b", "main")
        self.git("config", "user.name", "Release Test")
        self.git("config", "user.email", "release@example.invalid")
        self.git("config", "commit.gpgsign", "false")
        self.git("config", "tag.gpgSign", "false")
        self.git("remote", "add", self.remote, str(remote))


class PublishTest(unittest.TestCase):
    def test_version_order_and_stable_tag_selection(self):
        latest = publish.latest_stable_tag([
            "refs/tags/v0.0.9", "refs/tags/v0.0.10", "refs/tags/v0.1.0-rc.1",
            "refs/tags/v0.0.9^{}", "refs/tags/notes",
        ])
        self.assertEqual(publish.proposed_tag("patch", "0.0.9", latest), "v0.0.11")
        self.assertEqual(publish.proposed_tag("minor", "0.0.10", latest), "v0.1.0")
        self.assertEqual(publish.proposed_tag("major", "0.0.10", latest), "v1.0.0")
        for candidate in ["v0.0.10", "v0.1.0-rc.1", "v1.0.0; echo unexpected"]:
            with self.subTest(candidate=candidate), self.assertRaises(ValueError):
                publish.proposed_tag(candidate, "0.0.10", latest)

    def test_atomic_tag_and_scoped_resumable_pushes(self):
        with tempfile.TemporaryDirectory() as scratch:
            root = Path(scratch)
            work = root / "work"
            work.mkdir()
            repo = LocalRepository(work, root / "remote.git")
            (work / "Cargo.toml").write_text(
                '[workspace]\nmembers = ["app"]\nresolver = "2"\n[workspace.package]\nversion = "1.2.2"\n'
            )
            (work / "app/src").mkdir(parents=True)
            (work / "app/Cargo.toml").write_text(
                '[package]\nname = "release-fixture"\nversion.workspace = true\nedition = "2021"\n'
            )
            (work / "app/src/lib.rs").touch()
            (work / "Cargo.lock").write_text(
                'version = 4\n\n[[package]]\nname = "release-fixture"\nversion = "1.2.2"\n'
            )
            (work / "manifest.json").write_text("old\n")
            (work / "other.txt").write_text("existing\n")
            repo.git("add", "Cargo.toml", "Cargo.lock", "app", "manifest.json", "other.txt")
            repo.git("commit", "-qm", "Initial version")
            repo.push(f"HEAD:{repo.ref}")
            with patch.object(publish, "run_checks") as checks:
                publish.publish_tag(repo, "v1.2.3")
                checks.assert_called_once_with(work)
            previous = repo.git("rev-parse", "HEAD")
            self.assertEqual(publish.manifest_version(work), "1.2.3")
            self.assertEqual(repo.git("diff", "--name-only", "HEAD^", "HEAD"), "Cargo.lock\nCargo.toml")
            self.assertEqual(publish.remote_tag_commit(repo, "v1.2.3"), previous)
            self.assertEqual(repo.remote_refs(repo.ref)[repo.ref], previous)

            (work / "manifest.json").write_text("updated\n")
            (work / "other.txt").write_text("unrelated local edit\n")
            repo.git("add", "other.txt")
            message = "chore(bucket): fixture"
            expected = lambda ref: repo.git("show", f"{ref}:manifest.json") == "updated"
            # Model the commit being created before the previous push failed.
            repo.git("commit", "--only", "-qm", message, "--", "manifest.json")
            new = publish.push_bucket_change(repo, previous, "manifest.json", message, expected)
            self.assertEqual(repo.git("diff", "--name-only", previous, new), "manifest.json")
            self.assertEqual(repo.git("diff", "--cached", "--name-only"), "other.txt")
            self.assertEqual(repo.git("show", f"{new}:other.txt"), "existing")
            self.assertEqual(publish.push_bucket_change(repo, new, "manifest.json", message, expected), new)
            repo.git("commit", "-qm", "Unrelated commit")
            with self.assertRaisesRegex(ValueError, "Unrelated commits"):
                publish.push_bucket_change(repo, new, "manifest.json", message, expected)
            self.assertEqual(repo.remote_refs(repo.ref)[repo.ref], new)

    def test_release_requires_complete_assets_and_verifies_all_hashes(self):
        tag = "v1.2.3"
        with tempfile.TemporaryDirectory() as scratch:
            root = Path(scratch)
            source = root / "published"
            source.mkdir()
            destination = root / "download"
            destination.mkdir()
            for name in release.assets(tag):
                (source / name).write_bytes(name.encode())
            release.generate_manifest(tag, source, publish.REPOSITORY)
            files = {entry.name: entry.read_bytes() for entry in source.iterdir()}
            assets = [
                {"name": name, "browser_download_url": f"{publish.RELEASES}/download/{tag}/{name}",
                 "state": "uploaded", "size": len(data), "digest": f"sha256:{publish.sha256(source / name)}"}
                for name, data in files.items()
            ]
            metadata = {"tag_name": tag, "draft": False, "prerelease": False, "assets": assets[:-1]}
            deadline = time.monotonic() + 60

            def request(url, _deadline):
                if url.startswith("https://api.github.com"):
                    return io.BytesIO(json.dumps(metadata).encode())
                return io.BytesIO(files[url.split("/")[-1].split("?")[0]])

            with patch.object(publish, "request", request):
                with self.assertRaisesRegex(ValueError, "eight expected assets"):
                    publish.verify_release(tag, destination, deadline)
                self.assertEqual(list(destination.iterdir()), [])
                metadata["assets"] = assets
                publish.verify_release(tag, destination, deadline)
                name = next(iter(release.assets(tag)))
                files[name] = b"x" * len(files[name])
                with self.assertRaisesRegex(ValueError, "asset digest mismatch"):
                    publish.verify_release(tag, destination, deadline)

    def test_hyperscoop_resolution_prefers_configured_then_home_then_sibling(self):
        with tempfile.TemporaryDirectory() as scratch:
            root = Path(scratch)
            home = root / "home"
            repo = root / "src" / "superfile"
            sibling = root / "src" / "hyperscoop"
            configured = root / "configured"
            preferred = home / "demo" / "hyperscoop"
            for path in (home, repo, sibling, configured, preferred):
                path.mkdir(parents=True)
            self.assertEqual(
                publish.resolve_hyperscoop(configured / "explicit", home, repo, {}),
                configured / "explicit",
            )
            self.assertEqual(
                publish.resolve_hyperscoop(None, home, repo, {"SUPERFILE_HYPERSCOOP": str(configured)}),
                configured,
            )
            self.assertEqual(
                publish.resolve_hyperscoop(None, home, repo, {"SUPERFILE_HYPERSCOOP": ""}),
                preferred,
            )
            self.assertEqual(publish.resolve_hyperscoop(None, home, repo, {}), preferred)
            preferred.rmdir()
            (home / "demo").rmdir()
            self.assertEqual(publish.resolve_hyperscoop(None, home, repo, {}), sibling.resolve())
            sibling.rmdir()
            with self.assertRaisesRegex(ValueError, "Hyperscoop checkout not found"):
                publish.resolve_hyperscoop(None, home, repo, {})

    def test_missing_checkout_reports_the_path(self):
        missing = Path(tempfile.gettempdir()) / "superfile-publish-missing-checkout"
        self.assertFalse(missing.exists())
        with self.assertRaises(ValueError) as caught:
            publish.Repository(missing, "hyperscoop", "main")
        self.assertIn(str(missing.resolve()), str(caught.exception))

    def test_detached_checkout_asks_for_the_branch(self):
        with tempfile.TemporaryDirectory() as scratch:
            root = Path(scratch)
            work = root / "work"
            work.mkdir()
            repo = LocalRepository(work, root / "remote.git")
            (work / "README").write_text("x\n", encoding="utf-8")
            repo.git("add", "README")
            repo.git("commit", "-qm", "Initial")
            repo.git("checkout", "--detach")
            with self.assertRaisesRegex(ValueError, "Check out main"):
                publish.Repository(work, "SuperfileView", "main")

    def test_windows_archive_architecture_and_shortcuts(self):
        with tempfile.TemporaryDirectory() as scratch:
            path = Path(scratch) / "windows.zip"
            manifest = {"bin": "superfile.exe", "shortcuts": [["superfile.exe", "Superfile"]]}
            binary = bytearray(90)
            binary[:2] = b"MZ"
            struct.pack_into("<I", binary, 60, 64)
            binary[64:68] = b"PE\0\0"
            struct.pack_into("<H", binary, 88, 0x20B)
            for machine in (0x8664, 0x14C):
                struct.pack_into("<H", binary, 68, machine)
                with zipfile.ZipFile(path, "w") as archive:
                    archive.writestr("superfile.exe", bytes(binary))
                if machine == 0x8664:
                    publish.validate_windows_zip(path, manifest)
                    with self.assertRaises(KeyError):
                        publish.validate_windows_zip(path, {**manifest, "shortcuts": [["missing.exe", "Other"]]})
                else:
                    with self.assertRaisesRegex(ValueError, r"x64 PE32\+"):
                        publish.validate_windows_zip(path, manifest)


if __name__ == "__main__":
    unittest.main()
