import test from 'node:test';
import assert from 'node:assert/strict';
import {readFileSync} from 'node:fs';
const source=readFileSync(new URL('../../web/modules/deployments/queue-columns.js',import.meta.url),'utf8').replaceAll('export function ','function ');
const resize=new Function(source+'\nreturn resizeQueueColumns;')();
test('column resizing transfers width between neighbors and respects minimum widths',()=>{
 const widths=[30,250,240,210,90,122];
 assert.deepEqual(resize(widths,1,50),[30,300,190,210,90,122]);
 assert.deepEqual(resize(widths,1,999),[30,390,100,210,90,122]);
 assert.deepEqual(resize(widths,1,-999),[30,90,400,210,90,122]);
 assert.deepEqual(widths,[30,250,240,210,90,122]);
});
test('a poll already in flight cannot replace the table during column dragging',async()=>{
 const queue=readFileSync(new URL('../../web/modules/deployments/queue-view.js',import.meta.url),'utf8');
 const pollSource=queue.slice(queue.indexOf('function scheduleQueuePoll('),queue.indexOf('function controlAction('));
 let timer,resolve,rendered=0,notified=0;
 const poll=new Function('window','getDeploymentQueue','render','notifyQueue',pollSource+'\nreturn scheduleQueuePoll;')({setTimeout:callback=>{timer=callback}},()=>new Promise(r=>{resolve=r}),()=>rendered++,()=>notified++);
 const root={isConnected:true,dataset:{queueMountId:'m'},querySelector:()=>({})};
 const state={mountId:'m',adding:false,busyKey:null,resizingColumns:false,plans:[]};
 poll(root,state);const running=timer();state.resizingColumns=true;resolve({plans:[{id:'updated'}]});await running;
 assert.equal(rendered,0);assert.equal(notified,0);assert.deepEqual(state.plans,[]);
 state.resizingColumns=false;const resumed=timer();resolve({plans:[{id:'updated'}]});await resumed;
 assert.equal(rendered,1);assert.equal(notified,1);assert.equal(state.plans[0].id,'updated');
});
