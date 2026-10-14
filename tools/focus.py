"""Bring the game window (azimuth.exe) to the foreground; with "enter", also press Enter (the
launcher's Start). Windows only lets a process take the foreground right after input, so a
synthetic Alt press comes first."""
import ctypes, ctypes.wintypes as wt, sys, time

u32 = ctypes.WinDLL("user32")
k32 = ctypes.WinDLL("kernel32")
WNDENUMPROC = ctypes.WINFUNCTYPE(wt.BOOL, wt.HWND, wt.LPARAM)


def game_window():
    found = []

    def each(hwnd, _):
        pid = wt.DWORD()
        u32.GetWindowThreadProcessId(hwnd, ctypes.byref(pid))
        if u32.IsWindowVisible(hwnd):
            h = k32.OpenProcess(0x1000, False, pid.value)
            buf = ctypes.create_unicode_buffer(260)
            size = wt.DWORD(260)
            if h and k32.QueryFullProcessImageNameW(h, 0, buf, ctypes.byref(size)) and buf.value.lower().endswith("azimuth.exe"):
                found.append(hwnd)
            if h:
                k32.CloseHandle(h)
        return True

    u32.EnumWindows(WNDENUMPROC(each), 0)
    return found[0] if found else None


def focus():
    hwnd = game_window()
    if not hwnd:
        return False
    u32.keybd_event(0x12, 0, 0, 0)
    u32.keybd_event(0x12, 0, 2, 0)
    u32.ShowWindow(hwnd, 9)
    ok = u32.SetForegroundWindow(hwnd)
    return bool(ok) and u32.GetForegroundWindow() == hwnd


if __name__ == "__main__":
    print("focus", focus())
    if "enter" in sys.argv[1:]:
        time.sleep(0.5)
        u32.keybd_event(0x0D, 0, 0, 0)
        time.sleep(0.12)
        u32.keybd_event(0x0D, 0, 2, 0)
