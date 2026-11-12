// SPDX-License-Identifier: GPL-3.0-or-later
import test from 'node:test';
import assert from 'node:assert/strict';
import {readOnlyExec} from './pull-request-process.mjs';
test('subprocess failure boundary drops raw stdout stderr argv error and cause',()=>{
 const secret='SEED_PRIVATE_EMAIL_TOKEN_BODY';
 let observed;
 try{readOnlyExec('git',[secret],{},()=>{throw Object.assign(new Error(secret),{stdout:secret,stderr:secret,cause:new Error(secret),cmd:secret});});}catch(e){observed=e;}
 assert.equal(observed.message,'PR_M0_READ_FAILED');
 assert(!JSON.stringify(observed).includes(secret));
 assert(!String(observed).includes(secret));
});
import {anchor,nodePath,fold,style,landedPresent} from './pull-request-helpers.mjs';
test('strict majority uses deepest directory and deduplicates',()=>{
 assert.deepEqual(anchor(['src/a/x.rs','src/a/y.rs','tests/t.rs']),{path:'/src/a',reason:'strict-majority'});
 assert.equal(anchor(['src/a/x','src/a/x','other/y']).path,'/');
});
test('ties use common ancestor or region, incomplete stays unknown',()=>{
 assert.deepEqual(anchor(['src/a/x','src/b/y']),{path:'/src',reason:'strict-majority'});
 assert.equal(anchor(['src/x','tests/y']).reason,'root');
 assert.equal(anchor(['src/x'],false).reason,'unknown');
 assert.equal(anchor([]).reason,'unknown');
});
test('collision checks future paths and directory prefixes',()=>{
 assert.equal(nodePath('/src',1,new Set(['src/.pull-requests/future-file'])),'/src/.pull-requests-2/#1');
 assert.equal(nodePath('/src',1,new Set(['src/.pull-requests'])),'/src/.pull-requests-2/#1');
 assert.equal(nodePath('/',1,new Set(['pull requests/#1/nested']),true),'/pull requests-2/#1');
});
test('stream causal ordering and old open age survive idle and close/reopen',()=>{
 const events=[{path:'/p',time:100,created:10,kind:'opened',owner:'author'},{path:'/p',time:200,kind:'closed',actor:'closer'},{path:'/p',time:200,kind:'reopened',actor:'reopener'}];
 assert.equal(fold(events,150).get('/p').state,'open');
 const p=fold(events,200).get('/p');assert.equal(p.state,'open');assert.equal(p.owner,'author');assert.equal(p.generation,3);assert.equal(p.created,10);
 assert.equal(style(p.state,100*86400,0).ageDays,100);
 assert.equal(fold(events,100*86400).get('/p').state,'open');
 // A delayed close beam can be tagged with generation 2 and must not alter generation 3 truth.
 assert.notEqual(p.generation,2);
});
test('style is substitutable independently of lifecycle and means dim then highlight',()=>{
 const state=fold([{path:'/p',time:10,created:10,kind:'opened',owner:'a'},{path:'/p',time:20,kind:'merged',actor:'m'}],25);
 const before=JSON.stringify([...state]);
 assert(style('open',15,0).alpha<style('merged',15,0.5).alpha);
 assert.equal(style('merged',15,0.5).glyph,'M');assert(style('merged',15,0.5).flash);
 assert.equal(style('merged',15,2).visible,false);
 assert.notEqual(style('open',15,0,'alternate').alpha,style('open',15,0).alpha);
 assert.equal(JSON.stringify([...state]),before);
});
test('real provider closure companion after merge preserves merger state',()=>{
 const s=fold([{path:'/p',time:10,created:10,kind:'opened',owner:'author'},{path:'/p',time:20,kind:'merged',actor:'actual-merger'},{path:'/p',time:21,kind:'merge-closed',actor:'closer'}],22).get('/p');
 assert.equal(s.state,'merged');assert.equal(s.actor,'actual-merger');
});
test('only exact associated already-present paths flash, no mutation or guessed missing files',()=>{
 const present=new Set(['/src/a','/src/b']);const before=[...present];
 assert.deepEqual(landedPresent({exact:true,paths:['/src/a','/src/a','/missing']},present),['/src/a']);
 assert.deepEqual(landedPresent({exact:false,paths:['/src/a']},present),[]);
 assert.deepEqual([...present],before);
});
