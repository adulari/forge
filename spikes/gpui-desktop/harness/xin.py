"""Real input via XTest on the private Xvfb :87 (never the user's display)."""
import ctypes, time
X=ctypes.CDLL("libX11.so.6"); T=ctypes.CDLL("libXtst.so.6")
X.XOpenDisplay.restype=ctypes.c_void_p
D=X.XOpenDisplay(b":87"); assert D
d=ctypes.c_void_p(D)
def move(x,y): T.XTestFakeMotionEvent(d,-1,int(x),int(y),0); X.XFlush(d)
def click(x,y,btn=1):
    move(x,y); time.sleep(0.03); T.XTestFakeButtonEvent(d,btn,1,0); X.XFlush(d); time.sleep(0.03); T.XTestFakeButtonEvent(d,btn,0,0); X.XFlush(d)
def wheel(x,y,n,down=True):
    move(x,y)
    for _ in range(n):
        b=5 if down else 4
        T.XTestFakeButtonEvent(d,b,1,0); T.XTestFakeButtonEvent(d,b,0,0)
    X.XFlush(d)
def keysym(name): return X.XStringToKeysym(name.encode())
def key(name,mods=()):
    kc=X.XKeysymToKeycode(d,keysym(name))
    for m in mods: T.XTestFakeKeyEvent(d,X.XKeysymToKeycode(d,keysym(m)),1,0)
    T.XTestFakeKeyEvent(d,kc,1,0); X.XFlush(d)
    T.XTestFakeKeyEvent(d,kc,0,0)
    for m in reversed(mods): T.XTestFakeKeyEvent(d,X.XKeysymToKeycode(d,keysym(m)),0,0)
    X.XFlush(d)
def typechar(c):
    names={" ":"space",".":"period",",":"comma","\n":"Return"}
    key(names.get(c,c))

def focus_largest():
    """No WM on the private Xvfb: give the biggest mapped child of root the input focus so toolkits treat it as active."""
    import subprocess, re
    tree = subprocess.run(["xwininfo", "-root", "-tree", "-display", ":87"], capture_output=True, text=True).stdout
    best = None
    for m in re.finditer(r"(0x[0-9a-f]+) (?:\"[^\"]*\"|\(has no name\)):.*? (\d+)x(\d+)\+(-?\d+)\+(-?\d+)", tree):
        wid, w, h = int(m.group(1), 16), int(m.group(2)), int(m.group(3))
        if best is None or w * h > best[1]: best = (wid, w * h)
    X.XSetInputFocus(d, ctypes.c_ulong(best[0]), 1, 0); X.XFlush(d)
    return hex(best[0])
