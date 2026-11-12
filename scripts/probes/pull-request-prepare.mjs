#!/usr/bin/env node
// SPDX-License-Identifier: GPL-3.0-or-later
// Offline M0 preparation from public sanitized acquisition and local bare history.
import {readFileSync,writeFileSync} from 'node:fs';
import {resolve} from 'node:path';
import {readOnlyExec as execFileSync} from './pull-request-process.mjs';
import {anchor,nodePath,fold} from './pull-request-helpers.mjs';
function main(){
const scratch=resolve(process.argv[2]),gitDir=resolve(process.argv[3]);
const source=JSON.parse(readFileSync(resolve(scratch,'source.json'),'utf8'));
const git=args=>execFileSync('git',['-C',gitDir,...args],{encoding:'utf8',maxBuffer:64*1024*1024,env:{...process.env,GIT_NO_LAZY_FETCH:'1'}});
const order=git(['log','--reverse','--format=%H','--since=2024-01-01T00:00:00Z','--until=2024-12-31T23:59:59Z','FETCH_HEAD']).trim().split('\n');
const rank=new Map(order.map((sha,i)=>[sha,i]));
source.commits.sort((a,b)=>a.time-b.time||rank.get(a.sha)-rank.get(b.sha));
const pathsAt=sha=>{const pieces=git(['diff-tree','-m','--first-parent','--no-commit-id','--name-status','--no-renames','-z','-r','--root',sha]).split('\0').filter(Boolean);const paths=[];for(let i=0;i<pieces.length;i+=2){if(!pieces[i+1])throw new Error('Invalid git path');paths.push({kind:pieces[i][0],path:pieces[i+1]});}return paths;};
const originalCodeRecords=[];
for(const p of source.seed.files)originalCodeRecords.push({time:source.from,actor:'initial-code-context',kind:'A',path:'/'+p,seed:true});
for(const c of source.commits)for(const p of c.files)originalCodeRecords.push({time:c.time,actor:c.actor,kind:p.kind,path:'/'+p.path});
const allCode=new Set(originalCodeRecords.map(r=>r.path));
const aliases=new Map([...new Set(source.pulls.flatMap(p=>[p.author,p.mergedBy,...p.reviews.nodes.map(r=>r.author),...p.timelineItems.nodes.flatMap(t=>[t.actor,t.requestedReviewer])]).filter(Boolean))].sort().map((login,i)=>[login,'collaborator-'+String(i+1).padStart(3,'0')]));
const alias=(login,p,event)=>aliases.get(login)||'unresolved-'+p+'-'+event;
const seconds=iso=>iso?Math.floor(Date.parse(iso)/1000):null;
const proposals=[];
for(const p of source.pulls){
 const created=seconds(p.createdAt),owner=alias(p.author,p.number,'author');
 const events=[{time:created,kind:'opened',actor:owner,order:-1,created,owner}];
 p.timelineItems.nodes.forEach((t,i)=>{const kinds={ReopenedEvent:'reopened',ClosedEvent:'closed',MergedEvent:'merged',ReadyForReviewEvent:'ready',ReviewRequestedEvent:'review-requested'};let kind=kinds[t.__typename];if(kind==='closed'&&p.state==='MERGED'&&seconds(t.createdAt)>=seconds(p.mergedAt))kind='merge-closed';if(kind)events.push({time:seconds(t.createdAt),kind,actor:alias(t.__typename==='MergedEvent'?p.mergedBy:t.actor,p.number,i),order:i,id:t.id});});
 p.reviews.nodes.forEach((r,i)=>{const kinds={APPROVED:'approved',CHANGES_REQUESTED:'changes-requested',COMMENTED:'commented'};if(kinds[r.state]&&r.submittedAt)events.push({time:seconds(r.submittedAt),kind:kinds[r.state],actor:alias(r.author,p.number,'review-'+i),order:10000+i,id:r.id});});
 events.sort((a,b)=>a.time-b.time||a.order-b.order);
 const area=anchor(p.files.nodes.map(f=>f.path),p.files.complete&&p.files.nodes.length===p.changedFiles);
 const merge=events.find(e=>e.kind==='merged');
 const commit=source.commits.find(c=>c.sha===p.mergeCommit?.oid);
 const association=merge&&commit?{exact:true,sha:commit.sha,time:merge.time,paths:pathsAt(commit.sha).filter(f=>f.kind!=='D'&&p.files.nodes.some(pf=>pf.path===f.path)).map(f=>'/'+f.path)}:{exact:false,paths:[]};
 proposals.push({number:p.number,created,owner,events,area,association});
}
function render(mode){
 const sidecar=[],records=[...originalCodeRecords],models=[];
 for(const p of proposals){
  const path=nodePath(p.area.path,p.number,allCode,mode==='region');
  const events=p.events.filter(e=>e.time<source.to).map(e=>({...e,path}));
  const prefix=fold(events,source.from-1).get(path);
  const inYear=events.filter(e=>e.time>=source.from);
  if(prefix?.state!=='open'&&!inYear.length)continue;
  models.push({...p,path,events});
  sidecar.push(['PR',path,p.number,p.created,p.owner].join('\t'));
  for(const e of events)sidecar.push(['EV',path,e.time,e.kind,e.actor].join('\t'));
  if(p.association.exact)for(const landed of p.association.paths)sidecar.push(['LAND',path,p.association.time,landed].join('\t'));
  if(prefix?.state==='open')records.push({time:source.from,actor:p.owner,kind:'A',path,meta:'ev=pr-opened',pr:true,seed:true});
  for(const e of inYear.filter(e=>e.kind!=='merge-closed'))records.push({time:e.time,actor:e.actor,kind:e.kind==='opened'||e.kind==='reopened'?'A':'M',path,meta:e.kind==='merged'?'ev=pr-merged':e.kind==='closed'?'ev=pr-closed':e.kind==='opened'?'ev=pr-opened':'',pr:true});
 }
 records.sort((a,b)=>a.time-b.time); // stable, ordinary equal-time traversal order is unchanged
 const lines=records.map(r=>{if(/[\n\r\t|]/.test(r.path)||/[\n\r\t|]/.test(r.actor))throw new Error('Unsafe log label');return `${r.time}|${r.actor}|${r.kind}|${r.path}${r.meta?'||'+r.meta:''}`;});
 writeFileSync(resolve(scratch,mode+'.log'),lines.join('\n')+'\n');
 writeFileSync(resolve(scratch,mode+'.sidecar'),sidecar.join('\n')+'\n');
 return {mode,records:records.length,prRecords:records.filter(r=>r.pr).length,codeRecords:records.filter(r=>!r.pr).length,seedPrs:records.filter(r=>r.pr&&r.seed).length,models};
}
const anchored=render('anchors'),region=render('region');
writeFileSync(resolve(scratch,'plain.log'),originalCodeRecords.map(r=>`${r.time}|${r.actor}|${r.kind}|${r.path}`).join('\n')+'\n');
const inYear=p=>p.events.filter(e=>e.time>=source.from&&e.time<source.to);
const summary={repo:source.repo,year:source.year,commitYearDates:[new Date(Math.min(...source.commits.map(c=>c.time))*1000).toISOString(),new Date(Math.max(...source.commits.map(c=>c.time))*1000).toISOString()],completeCandidatePulls:source.pulls.length,createdInYear:proposals.filter(p=>p.created>=source.from&&p.created<source.to).length,reviewsInYear:proposals.reduce((n,p)=>n+inYear(p).filter(e=>['approved','changes-requested','commented'].includes(e.kind)).length,0),mergedInYear:proposals.filter(p=>inYear(p).some(e=>e.kind==='merged')).length,closedInYear:proposals.reduce((n,p)=>n+inYear(p).filter(e=>e.kind==='closed').length,0),reopenedInYear:proposals.reduce((n,p)=>n+inYear(p).filter(e=>e.kind==='reopened').length,0),anchorCounts:Object.fromEntries([...new Set(proposals.map(p=>p.area.path))].sort().map(path=>[path,proposals.filter(p=>p.area.path===path).length])),maxPrFileCount:Math.max(...source.pulls.map(p=>p.changedFiles)),exactAssociations:proposals.filter(p=>p.association.exact&&p.association.time>=source.from&&p.association.time<source.to).length,exactAssociatedPaths:proposals.filter(p=>p.association.exact&&p.association.time>=source.from&&p.association.time<source.to).reduce((n,p)=>n+p.association.paths.length,0),associationUnknownInYear:proposals.filter(p=>inYear(p).some(e=>e.kind==='merged')&&!p.association.exact).length,anchorRecords:anchored.records,regionRecords:region.records,codeRecords:anchored.codeRecords,seedPrs:anchored.seedPrs,initialTreeSeedCodeFiles:source.seed.files.length,renderedProposals:anchored.models.length,prRecords:anchored.prRecords,selectionRationale:'Among measured candidates and completed years, bat2024 has 49 distinct author names (53 raw author name/email identities, not 53 proven people), 332 reachable default-branch commits, 188 newly created PRs and substantial review/merge activity. Matches approximately-50 target more closely than measured 2025 alternatives and spans useful code/assets/test areas.',limits:['All 202 candidate PRs acquired, not sampled. Actual review connection rows are mapped only for supported submitted verdicts.','PR files are current/final snapshot paths. Exact merge SHA associations use default-history SHA and PR path intersection with first-parent merge diff; unknown stays unknown, no ref fetch.','Disposable geometry carrier uses M rather than deleting terminal PR nodes, so fold/style decides visibility while invisible terminal geometry can still influence layout. Review/ready/request beams remain generic M actions, no new production event kinds.','Initial code tree and old-open PRs are seeded as A geometry in existing engine, not actionless rich seeds. This creates current-engine context actions, not actual year activity.','Current engine file-idle-time=0 retains context. No positive-idle PR bypass, directory-delete protection, beam-generation protection, hide/filter/seek/F3 semantics or production v3 are implemented.','All bodies/emails/titles/patches omitted. Public logins stay source-local only; browser labels pseudonymized. No public histories/screenshots.']};
writeFileSync(resolve(scratch,'prepared.json'),JSON.stringify(summary,null,2));
writeFileSync(resolve(scratch,'models.json'),JSON.stringify(anchored.models));
console.log(JSON.stringify(summary,null,2));
}
try{main();}catch{console.error('PR_M0_PREPARE_FAILED');process.exitCode=1;}
