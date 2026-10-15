#!/usr/bin/env python3
"""Map RVAs of one azimuth.exe build to another: functions are matched by their instructions
with every address-dependent operand blanked (rip-relative displacements, call/jump targets);
code RVAs keep their offset inside the matched function, data RVAs follow the instruction that
references them (or the nearest referenced address below them, for slots of a table).

Usage (capstone: pip install capstone):
    tools/azimuth_map.py <old azimuth.exe> <new azimuth.exe> <rva> [<rva> ...]
Prints each RVA in the new build and how it was found; a table slot holding a code pointer is
checked against the matched function (OK / MISMATCH; "expect 0x0" means that function was not
matched on its own: compare it by hand)."""
import struct, sys, hashlib, collections
import capstone
from capstone import x86

class Image:
    def __init__(self, path):
        self.b = b = open(path, 'rb').read()
        o = struct.unpack_from('<I', b, 0x3c)[0]
        n = struct.unpack_from('<H', b, o + 6)[0]
        sz = struct.unpack_from('<H', b, o + 20)[0]
        self.base = struct.unpack_from('<Q', b, o + 24 + 24)[0]
        self.secs = [struct.unpack_from('<8sIIII', b, o + 24 + sz + i * 40) for i in range(n)]
        pd = self.sec('.pdata')
        self.funcs = sorted({struct.unpack_from('<II', b, pd[4] + i * 12) for i in range(pd[1] // 12)})
        self.md = capstone.Cs(capstone.CS_ARCH_X86, capstone.CS_MODE_64)
        self.md.detail = True
        self.prints = {}
        self.refs = collections.defaultdict(list)   # data rva -> [(func, offset of insn)]
        self.insns = {}
        for f, e in self.funcs:
            self.fingerprint(f, e)

    def sec(self, name):
        return next(s for s in self.secs if s[0].rstrip(b'\0').decode() == name)

    def section_of(self, rva):
        for nm, vs, va, rs, rp in self.secs:
            if va <= rva < va + max(vs, rs):
                return nm.rstrip(b'\0').decode()

    def off(self, rva):
        for nm, vs, va, rs, rp in self.secs:
            if va <= rva < va + rs:
                return rva - va + rp

    def read(self, rva, n):
        o = self.off(rva)
        return self.b[o:o + n] if o is not None else None

    def fingerprint(self, f, e):
        code = self.read(f, e - f)
        toks, insns = [], []
        for ins in self.md.disasm(code, f):
            ops = ins.op_str
            target = None
            for op in ins.operands:
                if op.type == x86.X86_OP_MEM and op.mem.base == x86.X86_REG_RIP:
                    target = ins.address + ins.size + op.mem.disp
                    ops = 'RIPREF'
            if ins.group(capstone.CS_GRP_CALL) or ins.group(capstone.CS_GRP_JUMP):
                if ins.operands and ins.operands[0].type == x86.X86_OP_IMM:
                    tgt = ins.operands[0].imm
                    ops = 'REL' if not (f <= tgt < e) else 'LOCAL%+d' % (tgt - f)
                    if not (f <= tgt < e):
                        insns.append((ins.address - f, 'call' if ins.group(capstone.CS_GRP_CALL) else 'jmp', tgt))
            if target is not None:
                self.refs[target].append((f, ins.address - f))
                insns.append((ins.address - f, 'ref', target))
            toks.append(ins.mnemonic + ' ' + ops)
        self.insns[f] = insns
        self.prints[f] = hashlib.sha1('\n'.join(toks).encode()).hexdigest() + ':%d' % (e - f)

    def func_of(self, rva):
        for f, e in self.funcs:
            if f <= rva < e:
                return f


def match(old, new):
    by_print = collections.defaultdict(list)
    for f, p in new.prints.items():
        by_print[p].append(f)
    old_by_print = collections.defaultdict(list)
    for f, p in old.prints.items():
        old_by_print[p].append(f)
    m = {}
    for p, fs in old_by_print.items():
        if len(fs) == 1 and len(by_print.get(p, [])) == 1:
            m[fs[0]] = by_print[p][0]
    # spread through calls of matched functions (same position, same kind)
    for _ in range(4):
        added = 0
        for f, g in list(m.items()):
            a, b = old.insns.get(f, []), new.insns.get(g, [])
            if len(a) != len(b):
                continue
            for (oa, ka, ta), (ob, kb, tb) in zip(a, b):
                if ka == kb == 'call' and oa == ob and ta not in m and old.func_of(ta) == ta and new.func_of(tb) == tb:
                    if old.prints.get(ta, '').split(':')[1:] == new.prints.get(tb, '').split(':')[1:]:
                        m[ta] = tb
                        added += 1
        if not added:
            break
    return m


def map_rva(old, new, m, rva):
    sec = old.section_of(rva)
    if sec == '.text':
        f = old.func_of(rva)
        if f in m:
            return m[f] + (rva - f), 'code: %s+%#x' % (hex(f), rva - f)
        return None, 'code: function %s not matched' % (hex(f) if f else '?')
    # data: an exact reference first, else the nearest referenced address below (a table slot)
    for tgt in sorted((t for t in old.refs if t <= rva and rva - t < 0x400), reverse=True):
        for f, o in old.refs[tgt]:
            if f in m:
                g = m[f]
                hit = [t for (oo, k, t) in new.insns.get(g, []) if oo == o and k == 'ref']
                if hit:
                    return hit[0] + (rva - tgt), 'data (%s): via %s at %s+%#x, slot +%#x' % (sec, hex(tgt), hex(f), o, rva - tgt)
    return None, 'data (%s): no matched reference' % sec


if __name__ == '__main__':
    old, new = Image(sys.argv[1]), Image(sys.argv[2])
    m = match(old, new)
    print('functions: old %d, new %d, matched %d' % (len(old.funcs), len(new.funcs), len(m)), file=sys.stderr)
    for a in sys.argv[3:]:
        rva = int(a, 16)
        r, why = map_rva(old, new, m, rva)
        extra = ''
        if r is not None and old.section_of(rva) != '.text':
            # a slot holding a code pointer: the new slot must hold the matched function
            ro, rn = old.read(rva, 8), new.read(r, 8)
            v_old = struct.unpack('<Q', ro)[0] - old.base if ro else None
            v_new = struct.unpack('<Q', rn)[0] - new.base if rn else None
            if v_old is not None and v_new is not None and old.section_of(v_old) == '.text':
                extra = ' check: holds %s -> expect %s, new holds %s %s' % (
                    hex(v_old), hex(m.get(v_old, 0)), hex(v_new),
                    'OK' if m.get(v_old) == v_new else 'MISMATCH')
        print('%-10s -> %-10s %s%s' % (a, hex(r) if r is not None else '?', why, extra))
