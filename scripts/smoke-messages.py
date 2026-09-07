#!/usr/bin/env python3
"""Exercise both CLIs with a synthetic iOS backup; build the workspace first.

Usage: python3 scripts/smoke-messages.py [--pdf] [--chrome-path PATH]
PDF uses Quartz on macOS unless --chrome-path is supplied. No personal backup
is needed. Temporary files are removed on success or failure.
"""
import argparse
import hashlib
import json
from pathlib import Path
import plistlib
import shutil
import sqlite3
import subprocess
import tempfile


def run(args):
    result = subprocess.run(list(map(str, args)), capture_output=True, text=True, timeout=180)
    assert result.returncode == 0, result.stderr + result.stdout
    return result.stdout


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--pdf', action='store_true')
    parser.add_argument('--chrome-path')
    args = parser.parse_args()
    repo = Path(__file__).resolve().parents[1]
    binaries = repo / 'target/debug'
    with tempfile.TemporaryDirectory(prefix='archive-messages-smoke-') as scratch:
        root = Path(scratch)
        backup = root / 'backup'
        backup.mkdir()
        lockdown = dict(BuildVersion='test', DeviceName='Synthetic iPhone',
                        ProductType='iPhone14,2', ProductVersion='27.0',
                        SerialNumber='TEST', UniqueDeviceID='TEST')
        (backup / 'Manifest.plist').write_bytes(plistlib.dumps(dict(
            IsEncrypted=False, Lockdown=lockdown, Applications={})))
        file_id = hashlib.sha1(b'HomeDomain-Library/SMS/sms.db').hexdigest()
        db = backup / file_id[:2] / file_id
        db.parent.mkdir()
        shutil.copyfile(repo / 'imessage-database/test_data/db/test.db', db)
        with sqlite3.connect(db) as conn:
            # Apple's triggers call private SQLite functions absent from Python.
            for (name,) in conn.execute("SELECT name FROM sqlite_master WHERE type='trigger'").fetchall():
                conn.execute('DROP TRIGGER "' + name.replace('"', '""') + '"')
            for table in ['message', 'attachment', 'message_attachment_join']:
                conn.execute(f'DELETE FROM {table}')
            for number in range(2):
                conn.execute('''INSERT INTO message
                    (guid, text, date, is_from_me, service, is_read, is_sent)
                    VALUES (?, ?, ?, 1, 'iMessage', 1, 1)''',
                    (f'smoke-{number}', f'Archive sync smoke {number}',
                     (700_000_000 + number * 3600) * 1_000_000_000))
        metadata = dict(LastModified=0, Flags=0, GroupID=501, LastStatusChange=0,
                        Birth=0, Size=db.stat().st_size, Mode=33188,
                        InodeNumber=1, ProtectionClass=1)
        blob = plistlib.dumps({'$top': {'root': plistlib.UID(1)},
                               '$objects': ['$null', metadata]}, fmt=plistlib.FMT_BINARY)
        with sqlite3.connect(backup / 'Manifest.db') as conn:
            conn.execute('CREATE TABLE Files (fileID TEXT, domain TEXT, relativePath TEXT, flags INTEGER, file BLOB)')
            conn.execute('INSERT INTO Files VALUES (?, ?, ?, 1, ?)',
                         (file_id, 'HomeDomain', 'Library/SMS/sms.db', blob))
        formats = ['txt', 'html'] + (['pdf'] if args.pdf else [])
        for fmt in formats:
            out = root / ('archive-' + fmt)
            command = [binaries / 'archive', '--backup', backup, '-o', out,
                       'messages', '-f', fmt]
            if fmt == 'pdf' and args.chrome_path:
                command += ['--chrome-path', args.chrome_path]
            envelope = json.loads(run(command))
            assert envelope['ok'] and envelope['command'] == 'messages'
            assert Path(envelope['summary']).is_file()
            transcripts = list(Path(envelope['output']).glob('*.' + fmt))
            assert transcripts, envelope
            for transcript in transcripts:
                if fmt == 'pdf':
                    assert transcript.read_bytes().startswith(b'%PDF-')
                else:
                    assert 'Archive sync smoke 0' in transcript.read_text()
            out = root / ('times-' + fmt)
            command = [binaries / 'imessage-exporter', '-a', 'iOS', '-p', backup,
                       '-f', fmt, '-o', out, '--use-message-times', '--no-progress']
            if fmt == 'pdf' and args.chrome_path:
                command += ['--chrome-path', args.chrome_path]
            run(command)
            transcripts = list(out.glob('*.' + fmt))
            assert transcripts
            for transcript in transcripts:
                stat = transcript.stat()
                assert stat.st_mtime == 978_307_200 + 700_003_600, stat
                if hasattr(stat, 'st_birthtime'):
                    assert stat.st_birthtime == 978_307_200 + 700_000_000, stat
            print(f'{fmt}: archive envelope, summary, transcript and exporter message dates OK')


if __name__ == '__main__':
    main()
