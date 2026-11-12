#!/usr/bin/env node
// SPDX-License-Identifier: GPL-3.0-or-later
// Read-only, public-only M0 acquisition. All evidence stays in caller's scratch directory.
import { readOnlyExec as execFileSync } from './pull-request-process.mjs';
import { mkdirSync, writeFileSync } from 'node:fs';
import { resolve } from 'node:path';
import { createHash } from 'node:crypto';
function main(){
const scratch = resolve(process.argv[2]);
const gitDir = resolve(process.argv[3]);
const repo = 'sharkdp/bat';
const from = Date.parse('2024-01-01T00:00:00Z') / 1000;
const to = Date.parse('2025-01-01T00:00:00Z') / 1000;
const asof = new Date().toISOString();
mkdirSync(scratch, {recursive:true});
const git = args => execFileSync('git', ['-C',gitDir,...args], {encoding:'utf8',maxBuffer:64*1024*1024,env:{...process.env,GIT_NO_LAZY_FETCH:'1'}});
const costs = [];
let calls=0, actualCost=0;
function graphql(query, variables={}) {
  if (++calls > 500 || actualCost >= 1000) throw new Error('Request/actual-cost probe cap reached');
  const raw = execFileSync('gh',['api','graphql','--input','-'],{input:JSON.stringify({query,variables}),encoding:'utf8',maxBuffer:16*1024*1024});
  const data=JSON.parse(raw);
  if(data.errors) throw new Error('GraphQL field error, output suppressed');
  costs.push(data.data.rateLimit);
  actualCost+=data.data.rateLimit.cost;
  if(actualCost>1000)throw new Error('Actual GraphQL cost exceeded 1000-point probe budget');
  if(data.data.rateLimit.remaining<100) throw new Error('Remaining-cost reserve reached');
  return data.data;
}
const meta=graphql('query { repository(owner:"sharkdp",name:"bat"){nameWithOwner isPrivate defaultBranchRef{name target{oid}}} rateLimit{cost remaining resetAt} }').repository;
if(meta.isPrivate) throw new Error('Public repository required');
const queries=[
  'repo:sharkdp/bat is:pr created:2024-01-01..2024-12-31',
  'repo:sharkdp/bat is:pr closed:2024-01-01..2024-12-31',
  'repo:sharkdp/bat is:pr created:<2025-01-01 closed:>=2025-01-01',
  'repo:sharkdp/bat is:pr created:<2025-01-01 is:open',
];
const candidates=new Map();
const searches=[];
for(const q of queries){
 let after=null,pages=0,total;
 do{
   const d=graphql('query($q:String!,$after:String){search(query:$q,type:ISSUE,first:100,after:$after){issueCount pageInfo{hasNextPage endCursor}nodes{... on PullRequest{number}}}rateLimit{cost remaining resetAt}}',{q,after});
   pages++; total=d.search.issueCount;
   if(total>1000) throw new Error('Search 1000 result cap reached');
   for(const p of d.search.nodes) candidates.set(p.number,p.number);
   after=d.search.pageInfo.hasNextPage?d.search.pageInfo.endCursor:null;
 }while(after);
 searches.push({query:q,pages,total});
}
const person = a => a?.login ?? null;
const pullFields='number id createdAt updatedAt closedAt mergedAt state isDraft changedFiles author{login} mergedBy{login} baseRefOid headRefOid mergeCommit{oid}';
const reviewsFields='totalCount pageInfo{hasNextPage endCursor} nodes{id submittedAt state author{login}}';
const filesFields='totalCount pageInfo{hasNextPage endCursor} nodes{path changeType}';
const timelineFields='totalCount pageInfo{hasNextPage endCursor} nodes{__typename ... on ReopenedEvent{id createdAt actor{login}} ... on ClosedEvent{id createdAt actor{login}} ... on MergedEvent{id createdAt actor{login} commit{oid}} ... on ReadyForReviewEvent{id createdAt actor{login}} ... on ReviewRequestedEvent{id createdAt actor{login} requestedReviewer{... on User{login}}}}';
const timelineArgs='itemTypes:[REOPENED_EVENT,CLOSED_EVENT,MERGED_EVENT,READY_FOR_REVIEW_EVENT,REVIEW_REQUESTED_EVENT]';
const pulls=[];
for(const number of [...candidates.keys()].sort((a,b)=>a-b)){
 const d=graphql(`query($n:Int!){repository(owner:"sharkdp",name:"bat"){pullRequest(number:$n){${pullFields} reviews(first:100){${reviewsFields}} files(first:100){${filesFields}} timelineItems(first:100,${timelineArgs}){${timelineFields}}}}rateLimit{cost remaining resetAt}}`,{n:number});
 const p=d.repository.pullRequest;
 for(const [key,fields,args] of [['reviews',reviewsFields,''],['files',filesFields,''],['timelineItems',timelineFields,','+timelineArgs]]){
   let conn=p[key],pages=1;
   while(conn.pageInfo.hasNextPage){
     const more=graphql(`query($n:Int!,$after:String!){repository(owner:"sharkdp",name:"bat"){pullRequest(number:$n){${key}(first:100,after:$after${args}){${fields}}}}rateLimit{cost remaining resetAt}}`,{n:number,after:conn.pageInfo.endCursor}).repository.pullRequest[key];
     p[key].nodes.push(...more.nodes); conn=more; pages++;
   }
   p[key].pages=pages;
   p[key].complete=p[key].nodes.length===p[key].totalCount;
 }
 // No title, body, email, patch, blob contents or token is requested or retained.
 p.author=person(p.author);p.mergedBy=person(p.mergedBy);
 for(const r of p.reviews.nodes)r.author=person(r.author);
 for(const t of p.timelineItems.nodes){t.actor=person(t.actor);if(t.requestedReviewer)t.requestedReviewer=person(t.requestedReviewer);}
 pulls.push(p);
 if(pulls.length%20===0)console.log(`JCODE_PROGRESS ${JSON.stringify({current:pulls.length,total:candidates.size,unit:'PRs',message:'Sanitized reviews/files/lifecycle acquisition'})}`);
}
const records=git(['log','--reverse','FETCH_HEAD','--since=2024-01-01T00:00:00Z','--until=2024-12-31T23:59:59Z','--format=%H%x09%ct%x09%aN%x09%aE']).trim().split('\n').map(s=>s.split('\t'));
const authorNames=new Set(records.map(r=>r[2]));
const authorPairs=new Set(records.map(r=>createHash('sha256').update(r[2]+'\0'+r[3]).digest('hex')));
const identities=[...authorPairs].sort();
const commits=[];
for(const [sha,t,name,email] of records){
 const actor='contributor-'+String(identities.indexOf(createHash('sha256').update(name+'\0'+email).digest('hex'))+1).padStart(2,'0');
 const rawPaths=git(['diff-tree','--no-commit-id','--name-status','--no-renames','-z','-r','--root',sha]).split('\0').filter(Boolean);
 const paths=[];
 for(let i=0;i<rawPaths.length;i+=2){const kind=rawPaths[i],path=rawPaths[i+1];if(!path)throw new Error('Invalid NUL-delimited git change');paths.push({kind:kind==='D'?'D':kind==='A'?'A':'M',path});}
 commits.push({sha,time:Number(t),actor,files:paths});
}
commits.sort((a,b)=>a.time-b.time); // Stable sort preserves original git traversal order at equal times.
const before=git(['rev-list','-1','--before=2024-01-01T00:00:00Z','FETCH_HEAD']).trim();
const seedFiles=git(['ls-tree','-r','--name-only','-z',before]).split('\0').filter(Boolean);
const source={repo,public:true,year:2024,from,to,asof,head:meta.defaultBranchRef.target.oid,defaultBranch:meta.defaultBranchRef.name,commits,seed:{sha:before,time:from,files:seedFiles},pulls};
writeFileSync(resolve(scratch,'source.json'),JSON.stringify(source));
const report={repo,year:2024,from:'2024-01-01T00:00:00Z',toExclusive:'2025-01-01T00:00:00Z',asof,head:source.head,commits:commits.length,commitsWithFiles:commits.filter(c=>c.files.length).length,changes:commits.reduce((n,c)=>n+c.files.length,0),initialTreeFiles:seedFiles.length,distinctAuthorNames:authorNames.size,distinctAuthorNameEmailPairs:authorPairs.size,contributorMethod:'Default-branch reachable commits by committer timestamp in UTC year. Distinct author names and name/email identities computed in memory, labels pseudonymized. Not unique-person resolver; no emails persisted.',pulls:pulls.length,reviews:pulls.reduce((n,p)=>n+p.reviews.nodes.length,0),files:pulls.reduce((n,p)=>n+p.files.nodes.length,0),timeline:pulls.reduce((n,p)=>n+p.timelineItems.nodes.length,0),searches,graphqlRequests:calls,graphqlActualCost:costs.reduce((n,c)=>n+c.cost,0),costStartRemaining:costs[0].remaining,costEndRemaining:costs.at(-1).remaining,reviewPages:pulls.reduce((n,p)=>n+p.reviews.pages,0),filePages:pulls.reduce((n,p)=>n+p.files.pages,0),timelinePages:pulls.reduce((n,p)=>n+p.timelineItems.pages,0),reviewsComplete:pulls.every(p=>p.reviews.complete),filesComplete:pulls.every(p=>p.files.complete&&p.files.nodes.length===p.changedFiles),timelineConnectionsComplete:pulls.every(p=>p.timelineItems.complete),coverageLimits:['Candidate set captures created/closed in year plus currently-open/post-year-closed pre-year-end proposals. Historic close/reopen cycles with no matching terminal timestamp can be missed. Full repository-level historic lifecycle completeness is not claimed.','Timeline item types restricted to supported close/reopen/merge/ready/review-request. Back-to-draft and review dismissal lifecycle unsupported.','Files are final/current PR snapshots, not historically exact opening diffs. Git default branch only, no PR refs fetched. No REST files fallback or 3000-cap exposure.','Git shallow-since 2023-01-01 retains all selected-year default ancestry but is not all repository refs. Initial tree is seeded context, not counted as year commit activity.']};
writeFileSync(resolve(scratch,'acquisition.json'),JSON.stringify(report,null,2));
console.log(JSON.stringify(report,null,2));
}
try { main(); } catch { console.error('PR_M0_ACQUIRE_FAILED'); process.exitCode=1; }
