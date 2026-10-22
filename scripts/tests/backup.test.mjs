import test from 'node:test';
import assert from 'node:assert/strict';
import {readFileSync} from 'node:fs';
const source=readFileSync(new URL('../../web/modules/backup/backup-view.js',import.meta.url),'utf8').replace(/^import .*;\n/gm,'').replace(/^export /gm,'');
function flow({password='source-workspace-password',confirm=true,path='/backup.gtbak'}={}) {
  const calls=[];
  const window={__TAURI__:{dialog:{save:async options=>{calls.push(['save',options]);return path;},open:async options=>{calls.push(['open',options]);return path;}}},confirm:()=>confirm};
  const summary={records:5,tables:2};
  const functions=new Function('window','exportBackup','importBackup','promptPassword',source+'\nrender=()=>{}; requestBackupPassword=promptPassword; return {exportWorkspace,importWorkspace};')(
    window,async(...args)=>{calls.push(['export',...args]);return summary;},async(...args)=>{calls.push(['import',...args]);return summary;},async()=>password);
  const state={busy:false,message:'',isError:false};
  return {calls,state,...functions};
}
test('export selects encrypted format without asking for or sending a password',async()=>{
  const f=flow();await f.exportWorkspace({},f.state);
  assert.equal(f.calls[0][1].defaultPath,'gatetree-ops-backup.gtbak');
  assert.deepEqual(f.calls[1],['export','/backup.gtbak']);
  assert.match(f.state.message,/Encrypted backup exported/);
});
test('import sends source workspace password, supports blank legacy passwords, and honors cancellation',async()=>{
  for (const password of ['source-workspace-password','']) {
    const f=flow({password});await f.importWorkspace({},f.state);
    assert.deepEqual(f.calls[1],['import','/backup.gtbak',password]);
    assert(!JSON.stringify(f.state).includes('source-workspace-password'));
  }
  for (const options of [{password:null},{confirm:false},{path:null}]) {
    const f=flow(options);await f.importWorkspace({},f.state);
    assert(!f.calls.some(([action])=>action==='import'));
  }
});
test('backup password dialog clears its field on submit, cancel and Escape',async()=>{
  for (const action of ['submit','cancel','escape']) {
    const handlers={};let removed=false;
    const input={value:'secret',focus(){}};
    const form={addEventListener:(name,handler)=>{handlers[name]=handler;}};
    const cancel={addEventListener:(name,handler)=>{handlers.cancel=handler;}};
    const dialog={returnValue:'',querySelector:selector=>selector==='input'?input:selector==='form'?form:cancel,
      addEventListener:(name,handler)=>{handlers[name]=handler;},showModal(){},remove(){removed=true;},close(value){this.returnValue=value;handlers.close();}};
    const document={createElement:()=>dialog};
    const request=new Function('document','requestAnimationFrame',source+'\nreturn requestBackupPassword;')(document,fn=>fn());
    const promise=request({append(){}});
    if (action==='submit') handlers.submit({preventDefault(){}});
    else if (action==='cancel') handlers.cancel();
    else handlers.close();
    assert.equal(await promise,action==='submit'?'secret':null);
    assert.equal(input.value,'');assert(removed);
  }
});
