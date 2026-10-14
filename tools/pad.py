"""Scripted controller for the game (hook adapters/azimuth_pad.c, shared memory Local\\FH2Online_Pad).

  python pad.py A B DOWN DOWN A          press buttons in order (150 ms each, 600 ms apart)
  python pad.py hold:RT:3000             hold a button/trigger for 3 s
  python pad.py stick:LX:-32767:500      push a stick for 0.5 s
  python pad.py wait:2000                pause
  python pad.py off                      give control back to the real pad
"""
import ctypes, ctypes.wintypes as wt, struct, sys, time

BUTTONS = {
    "UP": 0x0001, "DOWN": 0x0002, "LEFT": 0x0004, "RIGHT": 0x0008,
    "START": 0x0010, "MENU": 0x0010, "BACK": 0x0020, "VIEW": 0x0020, "LS": 0x0040, "RS": 0x0080,
    "LB": 0x0100, "RB": 0x0200, "A": 0x1000, "B": 0x2000, "X": 0x4000, "Y": 0x8000,
}
# active, sequence, buttons, lt, rt, lx, ly, rx, ry, reads
FMT = "<iiHBBhhhhi"

k32 = ctypes.WinDLL("kernel32", use_last_error=True)
k32.OpenFileMappingW.restype = wt.HANDLE
k32.MapViewOfFile.restype = ctypes.c_void_p
k32.MapViewOfFile.argtypes = [wt.HANDLE, wt.DWORD, wt.DWORD, wt.DWORD, ctypes.c_size_t]


class Pad:
    def __init__(self):
        h = k32.OpenFileMappingW(0x000F001F, False, "Local\\FH2Online_Pad")
        if not h:
            raise SystemExit("Local\\FH2Online_Pad not found: is the game running with hook fh2online.13+?")
        self.view = k32.MapViewOfFile(h, 0x000F001F, 0, 0, 4096)
        self.seq = struct.unpack_from(FMT, ctypes.string_at(self.view, 28))[1]
        self.state = dict(buttons=0, lt=0, rt=0, lx=0, ly=0, rx=0, ry=0)

    def reads(self):
        return struct.unpack_from(FMT, ctypes.string_at(self.view, 28))[9]

    def push(self, active=1):
        self.seq += 1
        s = self.state
        data = struct.pack("<iHBBhhhh", self.seq, s["buttons"], s["lt"], s["rt"], s["lx"], s["ly"], s["rx"], s["ry"])
        ctypes.memmove(self.view + 4, data, len(data))
        ctypes.memmove(self.view, struct.pack("<i", active), 4)

    def neutral(self):
        self.state = dict(buttons=0, lt=0, rt=0, lx=0, ly=0, rx=0, ry=0)
        self.push()

    def press(self, name, ms=150):
        self.hold(name, ms)
        time.sleep(0.6)

    def hold(self, name, ms):
        name = name.upper()
        if name in ("LT", "RT"):
            self.state[name.lower()] = 255
        else:
            self.state["buttons"] |= BUTTONS[name]
        self.push(); time.sleep(ms / 1000)
        self.neutral()

    def stick(self, axis, value, ms):
        self.state[axis.lower()] = value
        self.push(); time.sleep(ms / 1000)
        self.neutral()

    def off(self):
        self.neutral(); ctypes.memmove(self.view, struct.pack("<i", 0), 4)


def run(pad, args):
    for a in args:
        parts = a.split(":")
        cmd = parts[0].upper()
        if cmd == "OFF":
            pad.off()
        elif cmd == "WAIT":
            time.sleep(int(parts[1]) / 1000)
        elif cmd == "HOLD":
            pad.hold(parts[1], int(parts[2])); time.sleep(0.3)
        elif cmd == "STICK":
            pad.stick(parts[1], int(parts[2]), int(parts[3])); time.sleep(0.3)
        else:
            pad.press(cmd)


if __name__ == "__main__":
    p = Pad()
    p.neutral()
    run(p, sys.argv[1:])
    print("reads", p.reads())
