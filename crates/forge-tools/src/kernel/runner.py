import sys; del sys.path[0]
import os, io, ast, json, signal, subprocess, traceback, linecache, builtins, threading, queue, time, shlex
sys.path.insert(0, '')

NONCE = os.environ.pop('FORGE_KERNEL_NONCE', '')
for _k in [k for k in os.environ if k.startswith('FORGE_KERNEL_')]:
    os.environ.pop(_k, None)

ctrl_in = os.fdopen(os.dup(0), 'rb')
ctrl_out = os.fdopen(os.dup(1), 'wb')
_devnull = os.open(os.devnull, os.O_RDONLY)
os.dup2(_devnull, 0)
os.dup2(2, 1)
os.close(_devnull)
try:
    sys.stdout.reconfigure(line_buffering=True, errors='replace')
    sys.stderr.reconfigure(line_buffering=True, errors='replace')
except Exception:
    pass
try:
    with open('/proc/self/oom_score_adj', 'w') as _f:
        _f.write('1000')
except Exception:
    pass

state = {'running': False, 'bang': None}


def _on_sigint(signum, frame):
    if state['running']:
        raise KeyboardInterrupt


signal.signal(signal.SIGINT, _on_sigint)


def _flush():
    for s in (sys.stdout, sys.stderr):
        try:
            s.flush()
        except Exception:
            pass


def forge_sh(cmd):
    _flush()
    code = subprocess.run(cmd, shell=True, stdin=subprocess.DEVNULL).returncode
    if code != 0:
        print('[exit %d]' % code)
        state['bang'] = code
    return None


def _triple_state(line, open_q):
    i = 0
    n = len(line)
    while i < n:
        if open_q:
            if line[i] == '\\':
                i += 2
                continue
            if line.startswith(open_q, i):
                i += 3
                open_q = None
                continue
            i += 1
        else:
            c = line[i]
            if c == '#':
                break
            if c in '\'"':
                if line.startswith(c * 3, i):
                    open_q = c * 3
                    i += 3
                    continue
                j = i + 1
                while j < n and line[j] != c:
                    j += 2 if line[j] == '\\' else 1
                i = j + 1
                continue
            i += 1
    return open_q


def rewrite_magics(src):
    out = []
    open_q = None
    for line in src.split('\n'):
        if open_q is None:
            stripped = line.lstrip()
            indent = line[:len(line) - len(stripped)]
            if stripped.startswith('!'):
                line = indent + '__forge_sh(%r)' % stripped[1:].strip()
            elif stripped.startswith('%pip ') or stripped == '%pip':
                cmd = shlex.quote(sys.executable) + ' -m pip ' + stripped[4:].strip()
                line = indent + '__forge_sh(%r)' % cmd
        open_q = _triple_state(line, open_q)
        out.append(line)
    return '\n'.join(out)


ns = {'__name__': '__main__', '__builtins__': builtins, '__forge_sh': forge_sh}


def reply(obj):
    ctrl_out.write((json.dumps(obj) + '\n').encode('utf-8', 'replace'))
    ctrl_out.flush()


def run_cell(cid, code):
    fname = '<cell %d>' % cid
    state['bang'] = None
    t0 = time.monotonic()
    status, ename, evalue, tb, result = 'ok', None, None, None, None
    state['running'] = True
    try:
        src = rewrite_magics(code)
        linecache.cache[fname] = (len(src), None, src.splitlines(True), fname)
        tree = ast.parse(src, fname, 'exec')
        last = None
        if tree.body and isinstance(tree.body[-1], ast.Expr):
            last = ast.Expression(body=tree.body.pop().value)
        exec(compile(tree, fname, 'exec'), ns)
        if last is not None:
            value = eval(compile(last, fname, 'eval'), ns)
            if value is not None:
                ns['_'] = value
                result = repr(value)
                if len(result) > 8000:
                    result = result[:8000] + '... [repr truncated]'
    except KeyboardInterrupt:
        status = 'interrupted'
    except BaseException as e:
        status = 'error'
        ename = type(e).__name__
        evalue = str(e)
        t = e.__traceback__
        if t is not None and not isinstance(e, SyntaxError):
            t = t.tb_next
        tb = ''.join(traceback.format_exception(type(e), e, t))
    finally:
        state['running'] = False
    _flush()
    try:
        os.write(2, ('\x1eFORGE-END %s %d\x1e\n' % (NONCE, cid)).encode())
    except OSError:
        pass
    reply({'id': cid, 'status': status, 'result': result, 'ename': ename, 'evalue': evalue,
           'traceback': tb, 'elapsed_ms': int((time.monotonic() - t0) * 1000),
           'bang_exit': state['bang']})


inbox = queue.Queue()


def _reader():
    for raw in ctrl_in:
        inbox.put(raw)
    try:
        os.killpg(0, signal.SIGKILL)
    finally:
        os._exit(0)


threading.Thread(target=_reader, daemon=True).start()

while True:
    raw = inbox.get()
    try:
        msg = json.loads(raw)
    except ValueError:
        continue
    op = msg.get('op')
    if op == 'hello':
        reply({'status': 'ready', 'python': '%d.%d.%d' % sys.version_info[:3],
               'executable': sys.executable})
    elif op == 'exec':
        try:
            run_cell(msg['id'], msg['code'])
        except KeyboardInterrupt:
            state['running'] = False
    elif op == 'shutdown':
        break

try:
    os.killpg(0, signal.SIGKILL)
finally:
    os._exit(0)
