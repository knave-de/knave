#!/usr/bin/env python3
"""Exercise native quick settings against a frozen desktop and isolated command adapters."""
import argparse
import importlib.util
import json
import os
from pathlib import Path
import subprocess
import time
import tomllib
from gi.repository import Gio, GLib

CONTROL='org.knave.QuickSettings'
ADAPTER = r'''#!/usr/bin/python3
import fcntl,json,sys
from pathlib import Path
name=Path(sys.argv[0]).name
args=sys.argv[1:]
root=Path(sys.argv[0]).parent
with (root/'lock').open('a') as lock:
 fcntl.flock(lock,fcntl.LOCK_EX)
 path=root/'state.json'
 s=json.loads(path.read_text())
 result=''
 if name=='nmcli':
  if 'status' in args:
   result=('enpTest:ethernet:connected:Wired\n' if s['ethernet'] else 'enpTest:ethernet:disconnected:--\n')+('wlanTest:wifi:connected:Home\n' if s['wifi'] else 'wlanTest:wifi:disconnected:--\n')
  elif args==['radio','wifi']:result='enabled' if s['wifi'] else 'disabled'
  elif args[:2]==['radio','wifi']:s['wifi']=args[2]=='on'
  elif args[-2:]==['connection','show']:result='11111111-1111-1111-1111-111111111111:Wired:802-3-ethernet\n22222222-2222-2222-2222-222222222222:Home:802-11-wireless'
  elif 'disconnect' in args:s['ethernet']=False
  elif 'up' in args:s['ethernet']=args[-1].startswith('1');s['wifi']=True
 elif name=='wpctl':
  key='input' if 'SOURCE' in args[1] else 'output'
  if args[0]=='get-volume':result='Volume: %.2f%s'%(s[key]/100,' [MUTED]' if s[key+'_mute'] else '')
  elif args[0]=='set-volume':s[key]=int(args[2].rstrip('%'))
  elif args[0]=='set-mute':s[key+'_mute']=args[2]=='1'
 elif name=='brightnessctl':
  if args==['-m']:result='panel,backlight,800,%s%%,1000'%s['brightness']
  else:s['brightness']=int(args[-1].rstrip('%'))
 elif name=='bluetoothctl':
  if args==['show']:result='Controller Test\n\tPowered: '+('yes' if s['bluetooth'] else 'no')
  elif args[0]=='power':s['bluetooth']=args[1]=='on'
 elif name=='powerprofilesctl':
  if args==['get']:result=s['profile']
  elif args==['list']:result='  power-saver:\n* balanced:\n  performance:'
  else:s['profile']=args[1]
 elif name in ['pavucontrol','nm-connection-editor']:
  (root/(name+'.launched')).write_text('launched')
 else:raise RuntimeError(name)
 path.write_text(json.dumps(s))
 print(result)
'''


def wait(predicate, label, seconds=15):
    end=time.monotonic()+seconds
    while time.monotonic()<end:
        try:
            value=predicate()
            if value:return value
        except GLib.Error:
            pass
        time.sleep(.1)
    raise AssertionError(label)


def main():
    parser=argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--bundle',type=Path,required=True)
    parser.add_argument('--run',type=Path,required=True)
    parser.add_argument('--display',default='98')
    args=parser.parse_args()
    bundle,directory=args.bundle.resolve(),args.run.resolve()
    fixture=directory.with_name(directory.name+'-adapters')
    fixture.mkdir(parents=True)
    (fixture/'state.json').write_text(json.dumps(dict(ethernet=True,wifi=True,output=65,input=40,output_mute=False,input_mute=False,brightness=80,bluetooth=False,profile='balanced')))
    for tool in ['nmcli','wpctl','brightnessctl','bluetoothctl','powerprofilesctl','pavucontrol','nm-connection-editor']:
        path=fixture/tool
        path.write_text(ADAPTER)
        path.chmod(0o755)
    spec=importlib.util.spec_from_file_location('quick_settings_harness',Path(__file__).with_name('desktop-harness.py'))
    harness=importlib.util.module_from_spec(spec)
    spec.loader.exec_module(harness)
    def call(op,*values):return harness.request(directory,op,list(values))
    checks=[]
    def check(condition,label):
        if not condition:raise AssertionError(label)
        checks.append(label)
    def sample(pid):
        base=Path('/proc')/str(pid)
        fields=(base/'stat').read_text().split(') ',1)[1].split()
        status=dict(line.split(':',1) for line in (base/'status').read_text().splitlines() if ':' in line)
        return dict(ticks=int(fields[11])+int(fields[12]),rss_kib=int(status['VmRSS'].split()[0]),threads=int(status['Threads']),fds=len(list((base/'fd').iterdir())),switches=int(status['voluntary_ctxt_switches'])+int(status['nonvoluntary_ctxt_switches']),at=time.monotonic())
    started=False
    try:
        env=dict(os.environ,PATH=str(fixture)+':'+os.environ.get('PATH','/usr/bin'))
        subprocess.run(['python3',str(Path(__file__).with_name('desktop-harness.py')),'--run',str(directory),'start','--bundle',str(bundle),'--display',args.display,'--no-vnc'],env=env,check=True,capture_output=True,text=True,timeout=90)
        started=True
        session=call('status')['processes']['session']
        raw=Path(f'/proc/{session}/environ').read_bytes().split(b'\0')
        session_env=dict(entry.decode().split('=',1) for entry in raw if b'=' in entry)
        bus=Gio.DBusConnection.new_for_address_sync(session_env['DBUS_SESSION_BUS_ADDRESS'],Gio.DBusConnectionFlags.AUTHENTICATION_CLIENT|Gio.DBusConnectionFlags.MESSAGE_BUS_CONNECTION,None,None)
        def invoke(method,args=()):return bus.call_sync(CONTROL,'/org/knave/QuickSettings',CONTROL,method,GLib.Variant('(s)',args) if args else None,None,Gio.DBusCallFlags.NO_AUTO_START,15000,None).unpack()
        def state():return json.loads(invoke('Snapshot')[0])
        def command(action,**values):
            deadline=time.monotonic()+15
            while True:
                try:
                    return invoke('Command',(json.dumps(dict(action=action,**values)),))
                except GLib.Error as error:
                    if 'A system action is still in progress' not in str(error) or time.monotonic()>=deadline:
                        raise
                    time.sleep(.1)
        wait(lambda:state(),'service is available')
        children=[int(p) for p in Path(f'/proc/{session}/task/{session}/children').read_text().split()]
        def samples():return {str(pid):sample(pid) for pid in [session]+children if Path(f'/proc/{pid}').exists()}
        resources={'idle_before':samples()}
        time.sleep(3)
        resources['idle_after']=samples()
        call('record-start','quick-settings-demo')
        call('click',1248,18,1)
        wait(lambda:state()['open'] and state()['output'],'bar opens populated quick settings')
        check(state()['network']['ethernet']=='enpTest','Network prefers connected Ethernet')
        check(state()['output']['percent']==65 and state()['input']['percent']==40,'independent output and microphone state')
        time.sleep(.4)
        call('screenshot','quick-settings-main')
        # Mouse targets are verified against the native 1280x800 scene screenshots.
        # Service assertions below cover adapters without touching host hardware.
        command('set-volume',input=False,percent=72)
        command('set-mute',input=True,muted=True)
        check(state()['output']['percent']==72 and state()['input']['muted'],'volume and microphone commands reach isolated adapters')
        command('disconnect-ethernet')
        check(state()['network']['ethernet'] is None and state()['network']['wireless']=='Home','Wi-Fi fallback after Ethernet disconnect')
        command('connect',uuid='11111111-1111-1111-1111-111111111111')
        check(state()['network']['ethernet']=='enpTest','Ethernet restores preferred connection')
        command('set-wifi',enabled=False)
        check(not state()['network']['wifi'],'Wi-Fi radio control')
        command('set-wifi',enabled=True)
        command('set-brightness',percent=1)
        check(state()['brightness']==1,'brightness supports 1 percent')
        command('set-brightness',percent=55)
        command('set-bluetooth',enabled=True)
        command('set-power-profile',profile='power-saver')
        check(state()['brightness']==55 and state()['bluetooth'] and state()['power']['active']=='power-saver','brightness, Bluetooth and power profiles')
        for action,values in [('set-brightness',dict(percent=0)),('set-volume',dict(input=False,percent=101)),('set-charge-limit',dict(percent=49)),('set-night-light',dict(enabled=True,temperature=0,start=None,end=None)),('connect',dict(uuid='not-a-connection'))]:
            rejected=False
            try:command(action,**values)
            except GLib.Error:rejected=True
            check(rejected,'reject invalid '+action)
        command('set-night-light',enabled=True,temperature=3500,start=None,end=None)
        check(state()['night']['active'],'native Night Light enabled')
        time.sleep(.5)
        call('screenshot','night-light-warm')
        def blue(path):
            return float(subprocess.check_output(['magick',str(path),'-crop','300x30+880+50','-format','%[fx:mean.b]','info:'],text=True))
        check(blue(directory/'night-light-warm.png')<blue(directory/'quick-settings-main.png')*.8,'Night Light reduces rendered blue channel across the output')
        command('set-night-light',enabled=False,temperature=4000,start=None,end=None)
        check(not state()['night']['active'],'Night Light disabled')
        config=tomllib.loads((directory/'config.toml').read_text())
        check(config['quick_settings']['night_light']['temperature']==4000,'Night Light writes the isolated canonical configuration')
        call('key','Escape')
        wait(lambda:not state()['open'],'Escape closes quick settings')
        call('key','Super_L')
        wait(lambda:call('snapshot')['payload']['overview_visible'],'Overview opens')
        time.sleep(.8)
        call('screenshot','overview-before-settings')
        call('click',1248,18,1)
        wait(lambda:state()['open'] and not call('snapshot')['payload']['overview_visible'],'Overview hides before quick settings opens')
        check(True,'Overview quick-settings handoff')
        command('open-audio')
        wait(lambda:(fixture/'pavucontrol.launched').exists(),'pavucontrol launch')
        check(not state()['open'],'pavucontrol handoff closes panel')
        command('toggle')
        command('open-network')
        wait(lambda:(fixture/'nm-connection-editor.launched').exists(),'network editor launch')
        check(True,'additional network profiles launch nm-connection-editor')
        command('toggle')
        time.sleep(1)
        resources['normal']=samples()
        for _ in range(8):command('refresh')
        resources['stress']=samples()
        command('close')
        for pid,old in resources['idle_before'].items():
            new=resources['idle_after'].get(pid)
            if new:check(new['ticks']-old['ticks']<=5,'hidden process '+pid+' stays idle')
        for pid,old in resources['normal'].items():
            new=resources['stress'].get(pid)
            if new:check(new['threads']<=old['threads']+1 and new['fds']<=old['fds']+3,'bounded resources for '+pid)
        video=call('record-stop')
        report=dict(passed=True,checks=checks,resources=resources,video=video,hardware='Command adapters are isolated; physical devices, charge-limit authentication and DRM/KMS are not exercised.')
        (directory/'quick-settings-result.json').write_text(json.dumps(report,indent=2)+'\n')
        print(json.dumps(report,indent=2))
    finally:
        if started:call('stop')

if __name__=='__main__':main()
