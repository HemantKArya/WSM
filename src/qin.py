"""Guest input over QMP (usb-tablet + usb-kbd). Screen coords are in guest pixels (W x H).
usage: qin.py PORT click X Y | dclick X Y | type "text" | key ret|esc|tab|ctrl-a ... | move X Y
env: SCREEN=1920x1080"""
import json, os, socket, sys, time
W, H = map(int, os.environ.get("SCREEN", "1920x1080").split("x"))
s = socket.create_connection(("127.0.0.1", int(sys.argv[1]))); f = s.makefile("rwb")
f.readline(); f.write(b'{"execute":"qmp_capabilities"}\n'); f.flush(); f.readline()
def cmd(c, a):
    f.write(json.dumps({"execute": c, "arguments": a}).encode() + b"\n"); f.flush()
    while True:
        r = json.loads(f.readline())
        if "return" in r or "error" in r:
            if "error" in r: print(r)
            return r
def ev(events): cmd("input-send-event", {"events": events})
def move(x, y):
    ev([{"type": "abs", "data": {"axis": "x", "value": int(x * 32767 / (W - 1))}},
        {"type": "abs", "data": {"axis": "y", "value": int(y * 32767 / (H - 1))}}])
def click(x, y, n=1):
    move(x, y); time.sleep(0.15)
    for _ in range(n):
        ev([{"type": "btn", "data": {"down": True, "button": "left"}}]); time.sleep(0.08)
        ev([{"type": "btn", "data": {"down": False, "button": "left"}}]); time.sleep(0.08)
SHIFT = {'~': '`', '!': '1', '@': '2', '#': '3', '$': '4', '%': '5', '^': '6', '&': '7', '*': '8', '(': '9', ')': '0',
         '_': 'minus', '+': 'equal', '{': 'bracket_left', '}': 'bracket_right', '|': 'backslash', ':': 'semicolon',
         '"': 'apostrophe', '<': 'comma', '>': 'dot', '?': 'slash'}
NAMES = {' ': 'spc', '-': 'minus', '=': 'equal', '[': 'bracket_left', ']': 'bracket_right', chr(92): 'backslash',
         ';': 'semicolon', "'": 'apostrophe', ',': 'comma', '.': 'dot', '/': 'slash', '`': 'grave_accent', '\n': 'ret', '\t': 'tab'}
def keys(names, hold=0.03):
    cmd("send-key", {"keys": [{"type": "qcode", "data": k} for k in names], "hold-time": int(hold * 1000)}); time.sleep(0.03)
def typ(t):
    for ch in t:
        if ch.isupper(): keys(["shift", ch.lower()])
        elif ch in SHIFT: keys(["shift", {'~': 'grave_accent'}.get(ch, SHIFT[ch])])
        elif ch in NAMES: keys([NAMES[ch]])
        else: keys([ch])
a = sys.argv[2:]
if a[0] == "click": click(float(a[1]), float(a[2]))
elif a[0] == "dclick": click(float(a[1]), float(a[2]), 2)
elif a[0] == "move": move(float(a[1]), float(a[2]))
elif a[0] == "type": typ(a[1])
elif a[0] == "key":
    for k in a[1:]: keys(k.split("-"), 0.08)
