#!/usr/bin/env python3
"""Extract a verified updater into a new directory without following unsafe links."""
import pathlib
import shutil
import sys
import tarfile


def extract(archive_path, destination):
    destination = pathlib.Path(destination)
    if destination.exists():
        raise ValueError('Updater extraction requires a new destination')
    with tarfile.open(archive_path, 'r:gz') as archive:
        members = archive.getmembers()
        if not members:
            raise ValueError('Empty updater archive')
        for member in members:
            path = pathlib.PurePosixPath(member.name)
            if path.is_absolute() or '..' in path.parts or not path.parts or path.parts[0] != 'mimi.app':
                raise ValueError('Updater must contain only mimi.app')
            if not (member.isfile() or member.isdir()):
                raise ValueError('Unsupported updater archive entry')
        destination.mkdir()
        # This Rust bundle needs no links or special files. Copy ordinary files
        # into a new directory instead of trusting tar extraction paths/links.
        for member in members:
            target = destination / member.name
            if member.isdir():
                target.mkdir(parents=True, exist_ok=True)
            else:
                target.parent.mkdir(parents=True, exist_ok=True)
                with archive.extractfile(member) as source, target.open('xb') as output:
                    shutil.copyfileobj(source, output)
                target.chmod(member.mode & 0o777)


if __name__ == '__main__':
    if len(sys.argv) != 3:
        sys.exit('Usage: extract-macos-updater.py ARCHIVE NEW_DIRECTORY')
    extract(sys.argv[1], sys.argv[2])
