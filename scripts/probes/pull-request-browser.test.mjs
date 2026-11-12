// SPDX-License-Identifier: GPL-3.0-or-later
// Failure-path regression of the exact harness through synthetic VM modules.
// No browser, socket, API, build or performance workload is started.
import test from 'node:test';
import assert from 'node:assert/strict';
import {readFile} from 'node:fs/promises';
import * as vm from 'node:vm';
import {execFileSync} from 'node:child_process';
import {fileURLToPath} from 'node:url';
import * as realAssert from 'node:assert/strict';
import * as realPath from 'node:path';
import * as realCrypto from 'node:crypto';
const script=await readFile(new URL('./pull-request-browser.mjs',import.meta.url),'utf8');
async function failure(stage){
 const observed={listening:false,closed:false,browserLaunched:false,browserClosed:false,codes:[]};
 const source={from:0,to:365*86400};
 const models=[{number:1,path:'/src/.pull-requests/#1',created:1,owner:'a',area:{path:'/src'},association:{exact:true,time:100*86400,paths:['/src/a']},events:[{kind:'merged',time:100*86400,actor:'b'},{kind:'approved',time:99*86400}]}];
 const process={cwd:()=>'/workspace',argv:['node','probe','/scratch'],exitCode:0};
 const privateError=()=>new Error('SEEDED_PRIVATE_CHILD_MESSAGE');
 const browser={version:()=> 'synthetic',newPage:async()=>{throw privateError();},close:async()=>{observed.browserClosed=true;throw privateError();}};
 const modules={
  'node:assert/strict':realAssert,'node:path':realPath,'node:crypto':realCrypto,
  'node:module':{createRequire:()=>()=>({chromium:{launch:async()=>{observed.browserLaunched=true;if(stage==='launch')throw privateError();return browser;}}})},
  'node:http':{createServer:()=>({once:()=>{},listen:(_port,_host,cb)=>{observed.listening=true;cb();},close:cb=>{observed.closed=true;cb();}})},
  'node:fs/promises':{readFile:async p=>JSON.stringify(p.endsWith('models.json')?models:source),writeFile:async()=>{}},
  './pull-request-process.mjs':{readOnlyExec:()=>{if(stage==='metadata')throw privateError();return 'synthetic';}},
 };
 const context=vm.createContext({process,console:{error:code=>observed.codes.push(code),log:()=>{}},URL});
 const module=new vm.SourceTextModule(script,{context});
 await module.link(name=>{const exports=modules[name];assert(exports,'Unexpected harness import');return new vm.SyntheticModule(Object.keys(exports),function(){for(const [key,value] of Object.entries(exports))this.setExport(key,value);},{context});});
 await module.evaluate();await new Promise(resolve=>setImmediate(resolve));
 assert.deepEqual(observed.codes,['PR_M0_BROWSER_FAILED']);assert.equal(process.exitCode,1);
 return observed;
}
async function observe(stage){
 if(typeof vm.SourceTextModule==='function'&&typeof vm.SyntheticModule==='function')return failure(stage);
 // Default Node need not expose experimental VM exports. Each named test runs
 // one bounded self-child with the flag, never skips or recursively re-launches.
 let output;
 try{output=execFileSync(process.execPath,['--experimental-vm-modules',fileURLToPath(import.meta.url),'--pr-m0-vm-stage',stage],{encoding:'utf8',timeout:5000,maxBuffer:64*1024,stdio:['ignore','pipe','pipe']});}
 catch{throw new Error('PR_M0_VM_TEST_CHILD_FAILED');}
 return JSON.parse(output);
}
const childStage=process.argv[2]==='--pr-m0-vm-stage'?process.argv[3]:null;
if(childStage){
 try{
  assert(['metadata','launch','shutdown'].includes(childStage));
  // Direct child path cannot fall back again if the required VM API is absent.
  console.log(JSON.stringify(await failure(childStage)));
 }catch{console.error('PR_M0_VM_TEST_CHILD_FAILED');process.exitCode=1;}
}else{
 test('metadata failure is sanitized and never binds the listener',async()=>{
  const o=await observe('metadata');assert.equal(o.listening,false);assert.equal(o.browserLaunched,false);
 });
 test('launch failure closes an already-bound listener',async()=>{
  const o=await observe('launch');assert.equal(o.listening,true);assert.equal(o.closed,true);assert.equal(o.browserLaunched,true);
 });
 test('browser shutdown failure still closes the listener and suppresses private errors',async()=>{
  const o=await observe('shutdown');assert.equal(o.browserClosed,true);assert.equal(o.closed,true);
 });
}
