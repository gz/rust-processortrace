import re
import sys
import os


def parse_map(proc_map):
    # https://github.com/andikleen/simple-pt/blob/a5e33789d7f0eae71f3a7f89a2a54947838ba8d2/sptsideband.py#L102
    # https://github.com/andikleen/simple-pt/blob/a5e33789d7f0eae71f3a7f89a2a54947838ba8d2/dtools.c#L75
    lines = []
    for l in open(proc_map).readlines():
        m = re.match(r"""
        (?P<start>[0-9a-f]+)-(?P<end>[0-9a-f]+) \s+
        (?P<perm>\S+) \s+
        (?P<pgoff>[0-9a-f]+) \s+
        ([0-9a-f]+):([0-9a-f]+) \s+
        (?P<inode>\d+) \s+
        (?P<fn>.*)""", l, re.X)
        if not m:
            print >>sys.stderr, "no match", l,
            continue
        if not m.group('fn').startswith("/"):
            continue
        if m.group('perm').find('x') < 0:
            continue
        map_len = int(m.group('end'), 16) - int(m.group('start'), 16)
        lines.append(' '.join(["1", "1", sys.argv[2], m.group('start'), m.group(
            'pgoff'), format(map_len, 'x'), "\t" + m.group('fn')]))
    return '\n'.join(lines)


def ptinfo(f):
    return open(f).readlines()


if __name__ == '__main__':
    if len(sys.argv) < 3:
        print sys.argv[0], "<dump_files_prefix>", "<cr3 address>"
    else:
        print ''.join(ptinfo(sys.argv[1] + '.ptinfo')
                      ), parse_map(sys.argv[1] + '.ptmap')
