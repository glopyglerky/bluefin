import {test} from 'node:test';
import assert from 'node:assert/strict';
import vm from 'node:vm';
import {readFileSync} from 'node:fs';
const platformSource = readFileSync(new URL('./platform.js',import.meta.url),'utf8');
function fixture() {
  const events = {}, nodes = {}, docEvents = {}, winEvents = {};
  const video = {paused:true,duration:120,currentTime:0,audioTracks:[{label:'Stereo',enabled:false}],textTracks:[{language:'en',mode:'disabled'}],
    addEventListener:(e,f)=>events[e]=f,canPlayType:()=> 'maybe',play(){this.paused=false;return Promise.resolve();},pause(){this.paused=true;},removeAttribute(){delete this.src;},load(){}};
  nodes.video = video;
  const document = {hidden:false,getElementById:id=>nodes[id] ||= {textContent:''},addEventListener:(e,f)=>docEvents[e]=f};
  const window = {addEventListener:(e,f)=>winEvents[e]=f};
  const context = vm.createContext({window,document,URL,Promise,Number,Error}); vm.runInContext(platformSource,context);
  let errors = 0, ended = 0; const p = window.BluefinPlatform; p.init(()=>ended++,()=>errors++);
  return {p,video,document,window,context,nodes,events,docEvents,winEvents,counts:()=>({errors,ended})};
}
test('HTML player only gets safe HTTPS capability; cleanup and native failures',async () => {
  const f = fixture(), url = 'https://fixture.invalid/Bluefin/TV/v1/media/'+'a'.repeat(64)+'/0';
  await f.p.play(url); assert.equal(f.video.src,url); f.p.seek(999); assert.equal(f.video.currentTime,120);
  await f.p.pause(true); assert.equal(f.video.paused,true); await f.p.pause(false); assert.equal(f.video.paused,false);
  f.events.error(); f.events.ended(); assert.deepEqual(f.counts(),{errors:1,ended:1});
  f.p.selectTrack({type:'AUDIO',index:0}); assert.equal(f.video.audioTracks[0].enabled,true);
  f.p.selectTrack({type:'TEXT',index:0}); assert.equal(f.video.textTracks[0].mode,'showing');
  f.winEvents.pagehide(); assert.equal(f.video.src,undefined); assert.equal(f.video.paused,true);
  await assert.rejects(f.p.play(url+'?token=bad')); await assert.rejects(f.p.play(url.replace('https:','http:')));
  f.document.hidden=true; await assert.rejects(f.p.play(url)); await assert.rejects(f.p.handoff('https://youtube.com'));
});
test('shared remote geometry moves to nearest visible control and preserves input caret',() => {
  const f = fixture();
  function node(x,y,tag='BUTTON') {return {tagName:tag,disabled:false,getClientRects:()=>[1],getBoundingClientRect:()=>({left:x,top:y,width:40,height:40}),focus(){f.document.activeElement=this;},scrollIntoView(){}};}
  const left=node(0,0), right=node(70,0), down=node(0,80), hidden=node(45,0); hidden.getClientRects=()=>[];
  const controls = [left,right,down,hidden]; f.document.activeElement=left;
  f.document.querySelectorAll = selector=>selector==='[data-view]'?[]:controls;
  f.window.BluefinPolicy={load:()=>new Promise(()=>{})};
  f.nodes.now={addEventListener(){}};
  f.nodes.account={hidden:true};
  Object.assign(f.context,{localStorage:{getItem:()=> 'synthetic_device'},setInterval:()=>0,setTimeout:()=>0,clearTimeout:()=>{},getComputedStyle:()=>({visibility:'visible'})});
  vm.runInContext(readFileSync(new URL('../web/app.js',import.meta.url),'utf8'),f.context);
  let prevented=0;
  f.docEvents.keydown({key:'ArrowRight',target:left,preventDefault:()=>prevented++}); assert.equal(f.document.activeElement,right);
  left.focus(); f.docEvents.keydown({key:'ArrowDown',target:left,preventDefault:()=>prevented++}); assert.equal(f.document.activeElement,down);
  const input=node(0,0,'INPUT'); input.focus(); f.docEvents.keydown({key:'ArrowLeft',target:input,preventDefault:()=>prevented++}); assert.equal(f.document.activeElement,input); assert.equal(prevented,2);
  f.docEvents.keydown({key:'Escape',target:left,preventDefault:()=>{}}); assert.match(f.nodes.status.textContent,/Home/);
});
