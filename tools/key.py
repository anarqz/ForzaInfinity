"""Hold a keyboard key in the foreground window: python key.py F2 [ms=600]"""
import ctypes, sys, time
VK = {"F2": 0x71, "ESC": 0x1B, "ENTER": 0x0D}
u32 = ctypes.WinDLL("user32")
vk = VK[sys.argv[1].upper()]
u32.keybd_event(vk, 0, 0, 0)
time.sleep((int(sys.argv[2]) if len(sys.argv) > 2 else 600) / 1000)
u32.keybd_event(vk, 0, 2, 0)
