#!/usr/bin/env node
// SPDX-License-Identifier: GPL-3.0-or-later
// Local-only acceptance observation of actual WebGL2 engine and opt-in M0 adapter.
import assert from 'node:assert/strict';
import {createRequire} from 'node:module';
import {createServer} from 'node:http';
import {readFile,writeFile} from 'node:fs/promises';
import {resolve,join,extname} from 'node:path';
import {createHash} from 'node:crypto';
import {readOnlyExec} from './pull-request-process.mjs';
async function main(){
const root=process.cwd(),scratch=resolve(process.argv[2]);
const require=createRequire(join(root,'apps/player/package.json'));
const {chromium}=require('@playwright/test');
const models=JSON.parse(await readFile(join(scratch,'models.json'),'utf8'));
const source=JSON.parse(await readFile(join(scratch,'source.json'),'utf8'));
const choices=models.filter(p=>p.area.path!=='/'&&p.association.exact&&p.association.paths.length&&p.events.some(e=>e.kind==='merged'&&e.actor!==p.owner)&&p.association.time-p.created>2*86400&&p.association.time>source.from+70*86400&&p.association.time<source.from+280*86400&&p.events.some(e=>['approved','changes-requested','commented'].includes(e.kind)&&e.time<p.association.time));
choices.sort((a,b)=>b.association.paths.length-a.association.paths.length||a.number-b.number);
const target=choices[0];assert(target,'Actual code-anchored reviewed merged PR required');
const checkpoint={number:target.number,path:target.path,created:target.created,before:target.association.time-86400,merge:target.association.time,after:target.association.time+3600,associationPaths:target.association.paths};
const mime={'.html':'text/html','.js':'text/javascript','.wasm':'application/wasm','.log':'text/plain','.sidecar':'text/plain','.ttf':'font/ttf'};
const html=`<!doctype html><meta charset="utf-8"><title>Local PR M0 real engine</title><style>html,body{margin:0;background:#000;width:100%;height:100%;overflow:hidden}canvas{width:100vw;height:100vh;display:block}</style><canvas id="canvas"></canvas><script type="module">
const p=new URLSearchParams(location.search),module=p.get('baseline')==='1'?'/baseline/gource_web.js':'/wasm/gource_web.js';
const font=new FontFace('GourceSans','url(/fonts/FreeSans.ttf)');document.fonts.add(await font.load());await document.fonts.ready;
const {default:init,GourceApp,LogParser}=await import(module);await init();
const app=new GourceApp(document.querySelector('canvas'));window.app=app;app.resize(innerWidth,innerHeight,1);app.set_fixed_dt(1/60);
for(const [k,v] of Object.entries({'seconds-per-day':'0.3','disable-auto-skip':'1','camera-mode':'overview','filename-time':'2','font-size':'16','file-font-size':'12','hide':'filenames,usernames'})){app.set_option(k,v);}
const name=p.get('scenario')||'anchors';
if(p.get('probe')==='1'){app.probe_load(await (await fetch('/data/'+name+'.sidecar')).text());app.probe_variant(p.get('variant')||'dim-highlight');app.probe_focus(Number(p.get('focus')||0));}
const bytes=new Uint8Array(await (await fetch('/data/'+name+'.log')).arrayBuffer());const parser=new LogParser();app.begin_log();
for(const chunk of [parser.push(bytes),parser.finish()]){if(chunk.length)app.append_batch(chunk);}app.finish_log(bytes.length);
window.loaded={parserCommits:parser.commits(),skipped:parser.skipped_lines(),ignored:parser.meta_ignored(),bytes:bytes.length};parser.free();window.samples=[];window.ready=true;
window.tick=()=>{app.step(1);const s=JSON.parse(app.status());window.samples.push([s.last_sim_ms,s.last_build_ms,s.last_gpu_ms,s.draw_calls]);return s;};
</script>`;
const server=createServer(async(req,res)=>{
 try{
  const p=new URL(req.url,'http://127.0.0.1').pathname;
  if(p==='/'){res.setHeader('Content-Type','text/html');res.end(html);return;}
  if(p==='/favicon.ico'){res.writeHead(204).end();return;}
  let file;
  if(/^\/wasm\/gource_web(?:_bg)?\.(js|wasm)$/.test(p))file=join(scratch,p.slice(1));
  else if(/^\/baseline\/gource_web(?:_bg)?\.(js|wasm)$/.test(p))file=join(scratch,'baseline-wasm',p.split('/').at(-1));
  else if(/^\/data\/(anchors|region|plain)\.(log|sidecar)$/.test(p))file=join(scratch,p.split('/').at(-1));
  else if(p==='/fonts/FreeSans.ttf')file=join(root,'apps/player/assets/fonts/FreeSans.ttf');
  else{res.writeHead(404).end();return;}
  res.setHeader('Content-Type',mime[extname(file)]||'application/octet-stream');res.end(await readFile(file));
 }catch{res.writeHead(500).end('PR_M0_LOCAL_ASSET_FAILED');}
});
let browser;
const report={checkpoint,captures:[],plainParity:[],errors:[],externalRequests:0,hardware:readOnlyExec('sysctl',['-n','machdep.cpu.brand_string'],{encoding:'utf8'}).trim(),os:readOnlyExec('sw_vers',['-productVersion'],{encoding:'utf8'}).trim(),date:new Date().toISOString(),method:'Chrome headless real WASM/WebGL2, fixed dt 1/60, full sequential replay, GPU timings are submission CPU without forced GPU finish. Not real-time fps proof. Probe state consumes real frozen sidecar, actual positions through scratch-only exact path extraction. No static overlay.'};
const summary=a=>{const sorted=[...a].filter(Number.isFinite).sort((a,b)=>a-b);return {n:sorted.length,p50:sorted[Math.floor(sorted.length*.5)]||0,p95:sorted[Math.min(sorted.length-1,Math.floor(sorted.length*.95))]||0,max:sorted.at(-1)||0};};
async function newPage(width,height,query){
 const page=await browser.newPage({viewport:{width,height},deviceScaleFactor:1});
 page.on('pageerror',()=>{report.errors.push('PR_M0_PAGE_ERROR');});
 page.on('console',m=>{if(m.type()==='error')report.errors.push('PR_M0_CONSOLE_ERROR');});
 await page.route('**/*',route=>{if(new URL(route.request().url()).hostname!=='127.0.0.1'){report.externalRequests++;return route.abort();}return route.continue();});
 const start=Date.now();await page.goto('http://127.0.0.1:5176/?'+query);await page.waitForFunction(()=>window.ready,null,{timeout:120000});
 const loadMs=Date.now()-start;
 const gpu=await page.evaluate(()=>{const gl=document.querySelector('canvas').getContext('webgl2'),e=gl.getExtension('WEBGL_debug_renderer_info');return {vendor:gl.getParameter(e?e.UNMASKED_VENDOR_WEBGL:gl.VENDOR),renderer:gl.getParameter(e?e.UNMASKED_RENDERER_WEBGL:gl.RENDERER),version:gl.getParameter(gl.VERSION)};});
 const loaded=await page.evaluate(()=>window.loaded);assert.equal(loaded.skipped,0);assert.equal(loaded.ignored,0);
 return {page,loadMs,gpu,loaded};
}
async function until(page,time){
 let iterations=0;
 while(true){const done=await page.evaluate(time=>{for(let i=0;i<120;i++){const s=JSON.parse(app.status());if(s.time>=time||s.finished)return true;window.tick();}return false;},time);if(done)break;if(++iterations>600)throw Error('PR_M0_REPLAY_BOUND');}
}
async function capture(page,name,meta){
 const state=await page.evaluate(()=>({status:JSON.parse(app.status()),probe:app.probe_report?JSON.parse(app.probe_report()||'{}'):{}}));
 const path=join(scratch,name+'.png');await page.screenshot({path});
 assert(state.status.draw_calls>0&&state.status.draw_calls<=12);
 const readability=await page.evaluate(probe=>{
  const canvas=document.createElement('canvas'),ctx=canvas.getContext('2d');ctx.font='14px GourceSans';
  const rows=probe.rows.filter(r=>probe.shownNumbers?.includes(r.number));
  const rects=rows.map(r=>({number:r.number,x:r.number===probe.focusNumber&&probe.focusLabel?probe.focusLabel[0]:r.x,y:r.number===probe.focusNumber&&probe.focusLabel?probe.focusLabel[1]:r.y,width:ctx.measureText((r.state==='merged'?'M':r.state==='closed'?'X':'P')+' #'+r.number+' '+r.ageDays+'d +').width,height:14}));
  let overlaps=0,clipped=0;for(let i=0;i<rects.length;i++){const a=rects[i];if(a.x<0||a.y<0||a.x+a.width>innerWidth||a.y+a.height>innerHeight)clipped++;for(let j=i+1;j<rects.length;j++){const b=rects[j];if(a.x<b.x+b.width&&a.x+a.width>b.x&&a.y<b.y+b.height&&a.y+a.height>b.y)overlaps++;}}
  return {shown:rows.length,overlapPairs:overlaps,clipped,rects};
 },state.probe);
 const item={name,path,...meta,...state,readability};report.captures.push(item);console.log(`JCODE_PROGRESS ${JSON.stringify({message:'Captured '+name,labels:state.probe.labels,drawCalls:state.status.draw_calls,flash:state.probe.flashedPresentFiles})}`);
 return item;
}
try{
 await new Promise((r,reject)=>{server.once('error',reject);server.listen(5176,'127.0.0.1',r);});
 browser=await chromium.launch({channel:'chrome',headless:true,args:['--use-angle=metal','--ignore-gpu-blocklist','--enable-gpu']});
 report.browser=browser.version();
 // Plain log pixel parity between unchanged existing build and opt-in build with no sidecar.
 for(const baseline of [true,false]){
  const {page,gpu}=await newPage(1280,720,`scenario=plain&baseline=${baseline?'1':'0'}`);await page.evaluate(()=>{for(let i=0;i<420;i++)window.tick();});
  const pixels=await page.locator('canvas').screenshot();report.plainParity.push({baseline,sha256:createHash('sha256').update(pixels).digest('hex'),gpu,status:await page.evaluate(()=>JSON.parse(app.status()))});await page.close();
 }
 report.plainPixelIdentical=report.plainParity[0].sha256===report.plainParity[1].sha256;
 for(const width of [1280,390])for(const mode of ['anchors','region']){
  const {page,loadMs,gpu,loaded}=await newPage(width,width===390?800:720,`scenario=${mode}&probe=1&focus=${checkpoint.number}`);
  await until(page,checkpoint.before);const before=await capture(page,`${mode}-${width}-before`,{mode,width,loadMs,gpu,loaded});
  await until(page,checkpoint.after);const after=await capture(page,`${mode}-${width}-merge`,{mode,width,loadMs,gpu,loaded});
  await page.evaluate(()=>{app.probe_variant('uncapped');app.step(0);});await capture(page,`${mode}-${width}-uncapped`,{mode,width,variant:'uncapped'});
  await page.evaluate(()=>{app.probe_variant('dim-highlight');app.step(0);});
  await until(page,source.to-1);await capture(page,`${mode}-${width}-year-end`,{mode,width});
  const samples=await page.evaluate(()=>window.samples);const timing={sim:summary(samples.map(a=>a[0])),build:summary(samples.map(a=>a[1])),gpuSubmission:summary(samples.map(a=>a[2])),cpuFrame:summary(samples.map(a=>a[0]+a[1]+a[2])),drawCalls:summary(samples.map(a=>a[3]))};
  after.timings=timing;after.assertions={openBefore:before.probe.rows.some(r=>r.number===checkpoint.number&&r.state==='open'&&r.alpha===0.42),mergeAfter:after.probe.rows.some(r=>r.number===checkpoint.number&&r.state==='merged'&&r.alpha===1),landedPresentFlash:checkpoint.associationPaths.some(p=>after.probe.flashedPaths?.includes(p)),focusedLabelBefore:before.probe.shownNumbers.includes(checkpoint.number),focusedLabelAfter:after.probe.shownNumbers.includes(checkpoint.number),actualMergerOwnershipSeparate:after.probe.rows.some(r=>r.number===checkpoint.number&&r.actor!==r.owner),ageIsReplayClock:before.probe.rows.filter(r=>r.number===checkpoint.number).every(r=>r.ageDays===Math.floor((before.probe.replayTime-checkpoint.created)/86400))};
  await page.close();
 }
 assert.equal(report.externalRequests,0);assert.deepEqual(report.errors,[]);
 await writeFile(join(scratch,'browser.json'),JSON.stringify(report,null,2));
 console.log(JSON.stringify({captures:report.captures.map(c=>({name:c.name,path:c.path,labels:c.probe.labels,markers:c.probe.markers,calls:c.status.draw_calls,flash:c.probe.flashedPresentFiles,readability:c.readability,assertions:c.assertions})),plainPixelIdentical:report.plainPixelIdentical,errors:report.errors},null,2));
}finally{try{if(browser)await browser.close();}finally{await new Promise(r=>server.close(r));}}
}
main().catch(()=>{console.error('PR_M0_BROWSER_FAILED');process.exitCode=1;});
