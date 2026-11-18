default:
    @just --list

# Publish and sync Hyperscoop: just publish [patch|minor|major|vX.Y.Z].
publish version="patch":
    uv run --script scripts/package/publish.py publish {{quote(version)}}

# Inspect the proposed version and repository state without writing to Git.
publish-plan version="patch":
    uv run --script scripts/package/publish.py plan {{quote(version)}}

# Continue an existing release and finish verifying and synchronizing it.
publish-resume tag:
    uv run --script scripts/package/publish.py resume {{quote(tag)}}
