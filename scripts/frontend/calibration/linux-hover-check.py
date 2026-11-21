import subprocess,time,pathlib,json,os
root=pathlib.Path(os.environ['RUNNER_TEMP'])
source=pathlib.Path('scripts/frontend/check-caption-motion.mjs').read_text()
runner=pathlib.Path('scripts/frontend/check-caption-motion.mjs')
def processes():
 r=subprocess.run(['ps','-e','-o','pid=,ppid=,stat=,pcpu=,etimes=,wchan:24=,comm='],text=True,capture_output=True)
 return '\n'.join(l for l in r.stdout.splitlines() if any(w in l.lower() for w in ('chrome','xdg-','dbus','xvfb','ibus','gnome','vulkan','node','strace')))
assert 'browser = spawn(executable, ["--remote-debugging-pipe"' in source
for variant,headless,program in [('headless',True,'/usr/bin/google-chrome'),('x11-wrapper',False,'/usr/bin/google-chrome'),('x11-binary',False,'/opt/google/chrome/chrome')]:
 if not pathlib.Path(program).exists(): continue
 s=source.replace('browser = spawn(executable, ["--remote-debugging-pipe"','browser = spawn(executable, ['+('"--headless=new", ' if headless else '')+'"--remote-debugging-pipe"')
 s=s.replace('  assert.ok(pointer.hover && pointer.fine,','  assert.ok(pointer.hover && pointer.fine,')
 runner.write_text(s)
 f=root/f'chromium-{variant}.log'
 with f.open('w') as log:
  start=time.monotonic(); p=subprocess.Popen(['node',str(runner)],stdout=log,stderr=subprocess.STDOUT,env={**os.environ,'CHROME_BIN':program})
  for t in [1,5,10,14,18]:
   time.sleep(max(0,start+t-time.monotonic()))
   print(variant,t,processes(),flush=True)
   if p.poll() is not None or 'input-capabilities' in f.read_text():break
  try:p.wait(timeout=120)
  except subprocess.TimeoutExpired:p.terminate()
 print(variant,'exit',p.returncode, 'log',f.read_text()[-2900:],flush=True)
runner.write_text(source)
