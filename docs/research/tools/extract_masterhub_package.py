#!/usr/bin/env python3
"""Statically unpack a MasterHUB WiX v2 bundle; never run Windows code.

Requires Python 3.10+, pefile, and the native 7-Zip command-line executable.
Usage: python extract_masterhub_package.py --archive MasterHUB-Setup_1.0.0.113.zip \
       --sevenzip /path/to/7zz --output /path/to/new-analysis-directory
This deliberately supports the observed two-container bundle and two-byte MSI
string references. Unsupported formats fail explicitly rather than guessing.
"""
import argparse
import hashlib
import json
import os
from pathlib import Path, PurePosixPath
import shutil
import stat
import struct
import subprocess
import sys
import xml.etree.ElementTree as ET
import zipfile

import pefile

MAX_BYTES = 4 * 1024**3
MAX_HEADER = 32 * 1024**2


def require(condition, message):
    if not condition:
        raise ValueError(message)


def safe_relative(name):
    require(isinstance(name, str) and name and '\x00' not in name, 'Invalid empty/NUL path')
    name = name.replace('\\', '/')
    parts = name.split('/')
    require(not name.startswith('/') and ':' not in name and
            all(p not in ('', '.', '..') for p in parts), f'Unsafe relative path: {name!r}')
    return Path(*PurePosixPath(name).parts)


def digest(path, algorithm='sha256'):
    h = hashlib.new(algorithm)
    with path.open('rb') as stream:
        for block in iter(lambda: stream.read(1024 * 1024), b''):
            h.update(block)
    return h.hexdigest()


def write_json(path, data):
    path.parent.mkdir(parents=True, exist_ok=True)
    with path.open('x', encoding='utf-8') as stream:
        json.dump(data, stream, indent=2, ensure_ascii=False)
        stream.write('\n')


def extract(sevenzip, archive, destination):
    """Preflight names, links, duplicates and declared expansion size before 7zz."""
    listing = subprocess.run([str(sevenzip), 'l', '-slt', '-ba', '--', str(archive)],
                             check=True, capture_output=True, text=True).stdout
    names, total = set(), 0
    for block in listing.strip().split('\n\n'):
        fields = dict(line.split(' = ', 1) for line in block.splitlines() if ' = ' in line)
        if 'Path' not in fields:
            continue
        relative = safe_relative(fields['Path'])
        key = str(relative).casefold()
        require(key not in names, f'Duplicate archive entry: {relative}')
        names.add(key)
        require(not any('link' in k.lower() for k in fields), 'Archive links are unsupported')
        size = int(fields.get('Size', '0'))
        require(size >= 0, 'Negative archive entry size')
        total += size
        require(total <= MAX_BYTES, 'Archive expansion exceeds 4 GiB limit')
    require(names, f'Empty/unrecognized archive listing: {archive}')
    destination.mkdir(parents=True, exist_ok=False)
    result = subprocess.run([str(sevenzip), 'x', '-y', '-bd', '-bb0',
                             '-o' + str(destination), '--', str(archive)],
                            capture_output=True, text=True)
    (destination.parent / (destination.name + '-extraction.log')).write_text(
        result.stdout + result.stderr, encoding='utf-8')
    require(result.returncode == 0, f'7zz extraction failed: {archive}')
    for path in destination.rglob('*'):
        require(not path.is_symlink(), f'Extracted link: {path}')
        require(path.is_file() or path.is_dir(), f'Unsupported extracted object: {path}')
        require(path.resolve().is_relative_to(destination.resolve()), 'Extraction escaped destination')


def unpack_zip(archive, output):
    installer = output / 'installer'
    installer.mkdir()
    with zipfile.ZipFile(archive) as bundle:
        entries = [i for i in bundle.infolist() if not i.is_dir()]
        require(len(entries) == 1 and entries[0].filename.lower().endswith('.exe'),
                'Expected a ZIP containing exactly one installer EXE')
        info = entries[0]
        relative = safe_relative(info.filename)
        require(info.file_size <= MAX_BYTES, 'Installer is too large')
        mode = info.external_attr >> 16
        require(not stat.S_ISLNK(mode), 'ZIP symlinks unsupported')
        target = installer / relative
        target.parent.mkdir(parents=True, exist_ok=True)
        with bundle.open(info) as source, target.open('xb') as destination:
            shutil.copyfileobj(source, destination, 1024 * 1024)
        require(target.stat().st_size == info.file_size, 'ZIP size mismatch')
    return target


def carve_burn(installer, output):
    with pefile.PE(str(installer), fast_load=True) as pe:
        sections = [s for s in pe.sections if s.Name.rstrip(b'\x00') == b'.wixburn']
        require(len(sections) == 1, 'Expected one .wixburn section')
        section = sections[0]
        require(section.SizeOfRawData >= 56, 'Truncated Burn header')
        header = pe.__data__[section.PointerToRawData:section.PointerToRawData + 56]
    require(len(header) == 56, 'Burn section exceeds file bounds')
    words = struct.unpack('<14I', header)
    require(words[0] == 0x00F14300 and words[1] == 2 and words[10] == 1 and words[11] == 2,
            'Only WiX Burn v2, CAB format, two containers are supported')
    sizes = words[12:14]
    attached = words[8] + words[9] if words[8] else words[6] + sizes[0]
    offsets = [words[6], attached]
    require(offsets[1] >= offsets[0] + sizes[0], 'Overlapping Burn containers')
    container_dir = output / 'containers'
    container_dir.mkdir()
    records = []
    with installer.open('rb') as source:
        for index, (offset, size) in enumerate(zip(offsets, sizes)):
            require(size >= 36 and offset + size <= installer.stat().st_size, 'CAB exceeds EXE bounds')
            source.seek(offset)
            prefix = source.read(12)
            require(prefix[:4] == b'MSCF' and struct.unpack_from('<I', prefix, 8)[0] == size,
                    'CAB signature/length disagrees with Burn metadata')
            source.seek(offset)
            path = container_dir / f'container-{index}.cab'
            with path.open('xb') as destination:
                remaining = size
                while remaining:
                    block = source.read(min(1024 * 1024, remaining))
                    require(block, 'Truncated CAB')
                    destination.write(block)
                    remaining -= len(block)
            records.append({'path': path.relative_to(output).as_posix(), 'offset': offset, 'bytes': size})
    write_json(output / 'containers.json', records)
    return [output / r['path'] for r in records]


def map_burn_payloads(output):
    manifest = output / 'burn-bootstrap' / '0'
    require(manifest.stat().st_size <= MAX_HEADER, 'Burn XML is too large')
    raw = manifest.read_bytes()
    require(b'<!DOCTYPE' not in raw.upper() and b'<!ENTITY' not in raw.upper(), 'XML entities unsupported')
    root = ET.fromstring(raw)
    payloads = [p for p in root if p.tag.rsplit('}', 1)[-1] == 'Payload']
    package_dir = output / 'packages'
    package_dir.mkdir()
    records = []
    for payload in payloads:
        a = payload.attrib
        require(a.get('Packaging') == 'embedded', 'External Burn payload unsupported')
        source = output / 'burn-packages' / safe_relative(a['SourcePath'])
        target = package_dir / safe_relative(a['FilePath'])
        require(source.is_file() and source.stat().st_size == int(a['FileSize']), 'Burn payload size mismatch')
        if a.get('Hash'):
            require(digest(source, 'sha1').upper() == a['Hash'].upper(), 'Burn payload SHA-1 mismatch')
        target.parent.mkdir(parents=True, exist_ok=True)
        require(not target.exists(), 'Duplicate Burn destination')
        source.rename(target)
        records.append({'source_id': a['SourcePath'], 'path': target.relative_to(output).as_posix(),
                        'bytes': target.stat().st_size, 'sha256': digest(target)})
    write_json(output / 'burn-payload-map.json', records)
    msis = [output / r['path'] for r in records if r['path'].lower().endswith('.msi')]
    require(len(msis) == 1, 'Expected one MSI payload')
    return msis[0]


class MsiTables:
    """Observed MSI column-major streams with two-byte string references."""
    def __init__(self, root):
        self.root = root
        pool = (root / '!_StringPool').read_bytes()
        data = (root / '!_StringData').read_bytes()
        require(len(pool) >= 4 and len(pool) % 4 == 0, 'Invalid MSI string pool')
        codepage, flags = struct.unpack_from('<HH', pool)
        require(flags == 0, 'Extended MSI string references unsupported')
        codec = f'cp{codepage}' if codepage else 'utf-8'
        self.strings, position = [''], 0
        for offset in range(4, len(pool), 4):
            length, refs = struct.unpack_from('<HH', pool, offset)
            require(length or not refs, 'Long MSI strings unsupported')
            require(position + length <= len(data), 'MSI string exceeds data bounds')
            self.strings.append(data[position:position + length].decode(codec))
            position += length
        require(position == len(data), 'Unconsumed MSI string data')
        self.columns = {}
        raw = (root / '!_Columns').read_bytes()
        require(len(raw) % 8 == 0, 'Invalid MSI _Columns stream')
        count = len(raw) // 8
        for row in range(count):
            table, number, name, kind = [struct.unpack_from('<H', raw, (col * count + row) * 2)[0]
                                       for col in range(4)]
            number ^= 0x8000
            self.columns.setdefault(self.string(table), []).append([number, self.string(name), hex(kind)])
        for columns in self.columns.values():
            columns.sort()
            require([c[0] for c in columns] == list(range(1, len(columns) + 1)), 'Invalid MSI column order')

    def string(self, index):
        require(index < len(self.strings), 'MSI string reference out of bounds')
        return self.strings[index]

    def table(self, name):
        columns = self.columns[name]
        widths = [2 if int(c[2], 16) & 0x0800 else int(c[2], 16) & 0xFF for c in columns]
        require(all(w in (2, 4) for w in widths), f'Unsupported column width in {name}')
        data = (self.root / ('!' + name)).read_bytes()
        row_width = sum(widths)
        require(len(data) % row_width == 0, f'Truncated MSI table: {name}')
        rows = [{} for _ in range(len(data) // row_width)]
        start = 0
        for column, width in zip(columns, widths):
            for row, record in enumerate(rows):
                value = int.from_bytes(data[start + row * width:start + (row + 1) * width], 'little')
                record[column[1]] = (self.string(value) if int(column[2], 16) & 0x0800 else
                                     (value - (1 << (width * 8 - 1))) if value else None)
            start += len(rows) * width
        return rows


def installed_file_map(tables, output, sevenzip):
    components = {r['Component']: r for r in tables.table('Component')}
    directories = {r['Directory']: r for r in tables.table('Directory')}
    require('INSTALLFOLDER' in directories, 'Expected MSI INSTALLFOLDER root')
    cache = {'INSTALLFOLDER': []}

    def directory(key, visiting=()):
        require(key not in visiting and key in directories, 'MSI directory cycle/missing parent')
        if key not in cache:
            row = directories[key]
            name = row['DefaultDir'].split(':', 1)[0].split('|')[-1]
            parent = directory(row['Directory_Parent'], visiting + (key,))
            cache[key] = parent + ([] if name == '.' else [str(safe_relative(name))])
        return cache[key]

    raw = output / 'payload' / 'raw'
    raw.mkdir(parents=True)
    sources = {}
    for media in tables.table('Media'):
        cabinet = media['Cabinet']
        require(cabinet and not cabinet.startswith('#'), 'Embedded/absent MSI cabinet unsupported')
        cab = output / 'packages' / safe_relative(cabinet)
        require(cab.is_file(), f'Missing cabinet: {cabinet}')
        dest = raw / str(media['DiskId'])
        extract(sevenzip, cab, dest)
        for path in dest.rglob('*'):
            if path.is_file():
                require(path.name not in sources, 'Duplicate cabinet file identifier')
                sources[path.name] = path
    result, seen_paths = [], set()
    for record in tables.table('File'):
        file_id = record['File']
        require(file_id in sources, f'Missing cabinet file: {file_id}')
        source = sources[file_id]
        require(source.stat().st_size == record['FileSize'], f'MSI file size mismatch: {file_id}')
        require(record['Component_'] in components, 'Missing MSI component')
        name = str(safe_relative(record['FileName'].split('|')[-1]))
        installed = '/'.join(directory(components[record['Component_']]['Directory_']) + [name])
        require(installed.casefold() not in seen_paths, 'Duplicate installed path')
        seen_paths.add(installed.casefold())
        result.append(dict(record, installed_path=installed,
                           extracted_path=source.relative_to(output).as_posix(), sha256=digest(source)))
    require(len(sources) == len(result), 'Unmapped cabinet files')
    return result


def pe_inventory(path, installed_path, output):
    with pefile.PE(str(path), fast_load=True) as pe:
        pe.parse_data_directories(directories=[0, 1])
        exports = [{'name': s.name.decode('ascii', 'replace') if s.name else None,
                    'ordinal': s.ordinal, 'rva': hex(s.address),
                    'forwarder': s.forwarder.decode('ascii', 'replace') if s.forwarder else None}
                   for s in getattr(pe, 'DIRECTORY_ENTRY_EXPORT', ()).symbols] if hasattr(pe, 'DIRECTORY_ENTRY_EXPORT') else []
        imports = [{'dll': entry.dll.decode('ascii', 'replace'),
                    'functions': [item.name.decode('ascii', 'replace') if item.name else f'ordinal:{item.ordinal}'
                                  for item in entry.imports]}
                   for entry in getattr(pe, 'DIRECTORY_ENTRY_IMPORT', [])]
        return {'path': path.relative_to(output).as_posix(), 'installed_path': installed_path,
                'bytes': path.stat().st_size, 'sha256': digest(path),
                'machine': hex(pe.FILE_HEADER.Machine), 'image_base': hex(pe.OPTIONAL_HEADER.ImageBase),
                'managed': bool(pe.OPTIONAL_HEADER.DATA_DIRECTORY[14].VirtualAddress),
                'exports': exports, 'imports': imports}


def asar_inventory(path, installed_path):
    with path.open('rb') as source:
        prefix = source.read(16)
        require(len(prefix) == 16, 'Truncated ASAR prefix')
        size_marker, header_size, pickle_size, json_size = struct.unpack('<4I', prefix)
        require(size_marker == 4 and header_size == pickle_size + 4 and json_size <= pickle_size - 4 and
                8 + header_size <= path.stat().st_size and json_size <= MAX_HEADER, 'Invalid ASAR header')
        header = json.loads(source.read(json_size))
    count, unpacked = 0, 0

    def visit(node, depth=0):
        nonlocal count, unpacked
        require(depth < 128 and isinstance(node, dict), 'Invalid/deep ASAR tree')
        for name, entry in node.get('files', {}).items():
            safe_relative(name)
            if 'files' in entry:
                visit(entry, depth + 1)
            elif 'link' not in entry:
                count += 1
                require(isinstance(entry.get('size'), int) and entry['size'] >= 0, 'Invalid ASAR file size')
                if entry.get('unpacked'):
                    unpacked += 1
                else:
                    offset = int(entry['offset'])
                    require(offset >= 0 and 8 + header_size + offset + entry['size'] <= path.stat().st_size,
                            'ASAR file exceeds archive bounds')
    visit(header)
    return {'installed_path': installed_path, 'bytes': path.stat().st_size,
            'header_bytes': header_size, 'file_count': count, 'unpacked_file_count': unpacked}


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--archive', required=True, type=Path)
    parser.add_argument('--sevenzip', required=True, type=Path)
    parser.add_argument('--output', required=True, type=Path)
    args = parser.parse_args()
    archive, sevenzip, output = args.archive.resolve(), args.sevenzip.resolve(), args.output.resolve()
    require(archive.is_file() and sevenzip.is_file() and os.access(sevenzip, os.X_OK), 'Archive/tool missing')
    require(not output.exists() or (output.is_dir() and not any(output.iterdir())), 'Output must be new or empty')
    require(not archive.is_relative_to(output) and not sevenzip.is_relative_to(output), 'Inputs must be outside output')
    output.mkdir(parents=True, exist_ok=True)
    print('Extracting ZIP and WiX CAB containers...', flush=True)
    installer = unpack_zip(archive, output)
    cabs = carve_burn(installer, output)
    extract(sevenzip, cabs[0], output / 'burn-bootstrap')
    extract(sevenzip, cabs[1], output / 'burn-packages')
    msi = map_burn_payloads(output)
    extract(sevenzip, msi, output / 'msi-tables')
    tables = MsiTables(output / 'msi-tables')
    write_json(output / 'msi-strings.json', tables.strings)
    write_json(output / 'msi-columns.json', tables.columns)
    print('Mapping MSI files and inventorying PE/ASAR metadata...', flush=True)
    files = installed_file_map(tables, output, sevenzip)
    write_json(output / 'payload-file-map.json', files)
    pes, asars = [], []
    for record in files:
        path = output / record['extracted_path']
        with path.open('rb') as stream:
            prefix = stream.read(2)
        if prefix == b'MZ':
            pes.append(pe_inventory(path, record['installed_path'], output))
        if record['installed_path'].lower().endswith('.asar'):
            asars.append(asar_inventory(path, record['installed_path']))
    source = {'archive': {'name': archive.name, 'bytes': archive.stat().st_size, 'sha256': digest(archive)},
              'installer': {'path': installer.relative_to(output).as_posix(), 'bytes': installer.stat().st_size,
                            'sha256': digest(installer)}}
    write_json(output / 'source-manifest.json', source)
    write_json(output / 'pe-inventory.json', pes)
    write_json(output / 'asar-inventory.json', asars)
    write_json(output / 'analysis-manifest.json', {'schema_version': 1, 'source': source, 'payload_files': files,
        'extraction': {'method': 'ZIP / WiX Burn v2 / CAB / MSI tables', 'windows_code_executed': False,
                       'file_count': len(files), 'pe_count': len(pes), 'asar_count': len(asars)}})
    print(json.dumps({'file_count': len(files), 'pe_count': len(pes), 'asars': asars}, indent=2))


if __name__ == '__main__':
    try:
        main()
    except (ValueError, OSError, KeyError, ET.ParseError, zipfile.BadZipFile,
            subprocess.CalledProcessError, pefile.PEFormatError) as error:
        print(f'Inventory failed: {error}', file=sys.stderr)
        sys.exit(1)
