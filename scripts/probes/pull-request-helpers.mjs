// SPDX-License-Identifier: GPL-3.0-or-later
// Pure M0 helpers. No provider, backend, options codec or production lifecycle implementation.
export function anchor(paths, complete=true) {
  if(!complete || paths.length===0)return {path:'/',reason:'unknown'};
  const files=[...new Set(paths.map(p=>'/'+p.replace(/^\/+/,'')))];
  const counts=new Map();
  for(const p of files){const parts=p.split('/').filter(Boolean).slice(0,-1);for(let i=1;i<=parts.length;i++){const d='/'+parts.slice(0,i).join('/');counts.set(d,(counts.get(d)||0)+1);}}
  const majority=[...counts].filter(([,n])=>n>files.length/2).sort((a,b)=>b[0].split('/').length-a[0].split('/').length||a[0].localeCompare(b[0]));
  if(majority.length)return {path:majority[0][0],reason:'strict-majority'};
  const dirs=files.map(p=>p.split('/').filter(Boolean).slice(0,-1));
  const common=[];
  for(let i=0;i<dirs[0].length&&dirs.every(p=>p[i]===dirs[0][i]);i++)common.push(dirs[0][i]);
  return {path:'/'+common.join('/'),reason:common.length?'common-ancestor':'root'};
}
export function nodePath(anchorPath,number,allCodePaths,region=false){
  const root=region||anchorPath==='/'?'/pull requests':anchorPath;
  const code=[...allCodePaths].map(p=>'/'+p.replace(/^\/+/,''));
  for(let i=1;i<10000;i++){
    const dir=root+(root==='/pull requests'?'':`/.pull-requests${i===1?'':'-'+i}`)+(root==='/pull requests'&&i>1?'-'+i:'');
    const path=dir+'/#'+number;
    if(!code.some(p=>p===dir||dir.startsWith(p+'/')||p.startsWith(dir+'/')||p===path||path.startsWith(p+'/')||p.startsWith(path+'/')))return path;
  }
  throw new Error('No bounded collision-free marker directory');
}
export function fold(events,time){
 const state=new Map();
 for(const e of events){if(e.time>time||e.kind==='merge-closed')continue;const prev=state.get(e.path);state.set(e.path,{...e,created:prev?.created??e.created,generation:(prev?.generation??0)+1,state:e.kind==='merged'?'merged':e.kind==='closed'?'closed':e.kind==='opened'||e.kind==='reopened'?'open':prev?.state??'unknown',owner:prev?.owner??e.owner});}
 return state;
}
export function style(state,age,elapsed,variant='dim-highlight'){
 const visible=state==='open'||((state==='merged'||state==='closed')&&elapsed>=0&&elapsed<2);
 return {visible,alpha:state==='open'?(variant==='dim-highlight'?0.42:0.68):state==='merged'?1:0.35,scale:state==='merged'?1.8:1.35,glyph:state==='merged'?'M':state==='closed'?'X':'P',ageDays:Math.floor(Math.max(0,age)/86400),flash:state==='merged'&&elapsed>=0&&elapsed<2};
}
export function landedPresent(association,present){return association?.exact?[...new Set(association.paths)].filter(p=>present.has(p)):[];}
