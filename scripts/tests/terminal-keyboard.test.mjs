import test from 'node:test';
import assert from 'node:assert/strict';
import {readFileSync} from 'node:fs';
const source = readFileSync(new URL('../../web/modules/connections/terminal-keyboard.js', import.meta.url),'utf8');
const {configureTerminalKeyboard} = await import(`data:text/javascript;base64,${Buffer.from(source).toString('base64')}`);
function setup(platform='MacIntel') {
  const sent=[];
  const terminal={options:{},attachCustomKeyEventHandler(handler){this.handler=handler;},input(text,userInput){sent.push({text,userInput});}};
  configureTerminalKeyboard(terminal,platform);
  function key(values={}) {
    const event={type:'keydown',key:'|',code:'KeyI',keyCode:73,altKey:true,ctrlKey:false,metaKey:false,isComposing:false,defaultPrevented:false,preventDefault(){this.defaultPrevented=true;},...values};
    const result=terminal.handler(event);
    return {event,result};
  }
  return {terminal,sent,key};
}
test('Mac Option characters use the current keyboard layout and UTF-8 input, without a keypress',()=>{
  const {terminal,sent,key}=setup();
  for (const character of ['|','[',']','{','}','@','\\','€','í','😀']) {
    const {event,result}=key({key:character});
    assert.equal(result,false);assert.equal(event.defaultPrevented,true);
  }
  assert.deepEqual(sent.map(v=>new TextDecoder().decode(new TextEncoder().encode(v.text))),['|','[',']','{','}','@','\\','€','í','😀']);
  assert(sent.every(v=>v.userInput));
  assert.equal(terminal.options.macOptionIsMeta,false);
});
test('a following keypress cannot duplicate the Option character and repeats remain usable',()=>{
  const {sent,key}=setup();
  key();assert.equal(key({type:'keypress'}).result,false);
  assert.equal(sent.length,1);
  key({repeat:true});assert.equal(sent.length,2);
  key({type:'keyup'});
  assert.equal(key({type:'keypress',key:'x',altKey:false}).result,true);
  assert.equal(key({key:'x',altKey:false}).result,true);
});
test('dead keys, IME, shortcuts and terminal navigation keep their native handling',()=>{
  const {sent,key}=setup();
  for (const values of [{key:'Dead'},{key:'Process'},{key:'Unidentified'},{key:'ArrowLeft',keyCode:37},{key:'Backspace',keyCode:8},{key:'Tab',keyCode:9},{key:'\u001b'},{ctrlKey:true},{metaKey:true},{isComposing:true},{keyCode:229},{altKey:false}]) {
    const {event,result}=key(values);assert.equal(result,true);assert.equal(event.defaultPrevented,false);
  }
  assert.equal(sent.length,0);
});
test('Windows AltGr and Linux Alt behavior are unchanged',()=>{
  for (const platform of ['Win32','Linux x86_64']) assert.equal(setup(platform).terminal.handler,undefined);
});
