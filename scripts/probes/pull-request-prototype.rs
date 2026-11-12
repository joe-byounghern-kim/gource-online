// SPDX-License-Identifier: GPL-3.0-or-later
//! Disposable M0 scene adapter, opt-in only through probe_load. Not M1 lifecycle implementation.
//! Real engine positions and existing quad/text passes, no screen overlay or code mutations.
use gource_core::view::{SceneView,LabelView,LabelAnchor,FontRole};
use std::collections::BTreeMap;
#[derive(Clone,Debug)]
struct Event { time:i64, kind:String, actor:String }
#[derive(Clone,Debug)]
struct Proposal { path:String, number:u32, created:i64, owner:String, events:Vec<Event> }
#[derive(Clone,Debug)]
struct Land { proposal:String, time:i64, path:String }
#[derive(Default)]
pub struct Probe { proposals:Vec<Proposal>, lands:Vec<Land>, variant:String, focus:u32, report:String, previous_time:i64, terminal_started:BTreeMap<(String,i64),f32> }
#[derive(Clone,Debug,PartialEq)]
struct State { kind:String, event_time:i64, actor:String, generation:u32, verdict:String }
fn fold(p:&Proposal,time:i64)->State {
 let mut s=State{kind:"unknown".into(),event_time:p.created,actor:p.owner.clone(),generation:0,verdict:String::new()};
 for e in &p.events {
  if e.time>time || e.kind=="merge-closed" {continue;}
  s.generation+=1;s.actor=e.actor.clone();
  match e.kind.as_str(){
   "opened"|"reopened"=>{s.kind="open".into();s.event_time=e.time;},"merged"=>{s.kind="merged".into();s.event_time=e.time;},"closed"=>{s.kind="closed".into();s.event_time=e.time;},
   "approved"=>s.verdict="+".into(),"changes-requested"=>s.verdict="!".into(),"commented"=>s.verdict="C".into(),_=>{}
  }
 }
 s
}
#[derive(Clone,Copy,Debug,PartialEq)]
struct Visual { visible:bool, alpha:f32, scale:f32, glyph:&'static str, flash:bool }
trait Style {fn map(&self,state:&State,elapsed:f32)->Visual;}
struct DimHighlight;
impl Style for DimHighlight {fn map(&self,s:&State,elapsed:f32)->Visual{
 Visual{visible:s.kind=="open"||((s.kind=="merged"||s.kind=="closed")&&(0.0..2.0).contains(&elapsed)),alpha:if s.kind=="open"{0.42}else if s.kind=="merged"{1.0}else{0.35},scale:if s.kind=="merged"{1.8}else{1.35},glyph:if s.kind=="merged"{"M"}else if s.kind=="closed"{"X"}else{"P"},flash:s.kind=="merged"&&(0.0..2.0).contains(&elapsed)}
}}
struct Alternative;
impl Style for Alternative {fn map(&self,s:&State,elapsed:f32)->Visual{let mut v=DimHighlight.map(s,elapsed);if s.kind=="open"{v.alpha=0.68;}v}}
impl Probe {
 pub fn load(&mut self,text:&str)->Result<(),String>{
  if text.len()>16*1024*1024{return Err("Probe sidecar exceeds bound".into());}
  let mut next=Probe{variant:"dim-highlight".into(),..Probe::default()};
  for line in text.lines(){let p:Vec<&str>=line.split('\t').collect();match p.first().copied(){
   Some("PR") if p.len()==5=>{next.proposals.push(Proposal{path:p[1].into(),number:p[2].parse().map_err(|_|"number")?,created:p[3].parse().map_err(|_|"created")?,owner:p[4].into(),events:Vec::new()});},
   Some("EV") if p.len()==5=>{let pr=next.proposals.iter_mut().find(|x|x.path==p[1]).ok_or("orphan event")?;pr.events.push(Event{time:p[2].parse().map_err(|_|"time")?,kind:p[3].into(),actor:p[4].into()});},
   Some("LAND") if p.len()==4=>{next.lands.push(Land{proposal:p[1].into(),time:p[2].parse().map_err(|_|"time")?,path:p[3].into()});},
   _=>return Err("Malformed probe sidecar".into()),
  }}
  if next.proposals.len()>1000||next.lands.len()>100000{return Err("Probe record bound".into());}
  *self=next;Ok(())
 }
 pub fn active(&self)->bool{!self.proposals.is_empty()}
 pub fn materialize_paths(&self,now:i64,runtime:f32)->Vec<String>{
  self.proposals.iter().filter(|p|{let s=fold(p,now);let elapsed=self.terminal_started.get(&(p.path.clone(),s.event_time)).map(|start|runtime-start).unwrap_or(999.0);s.kind=="open"||((s.kind=="merged"||s.kind=="closed")&&(self.previous_time<s.event_time||DimHighlight.map(&s,elapsed).visible))}).map(|p|p.path.clone()).collect()
 }
 pub fn focus(&mut self,number:u32){self.focus=number;}
 pub fn variant(&mut self,name:&str){self.variant=name.into();}
 pub fn report(&self)->String{self.report.clone()}
 pub fn apply(&mut self,view:&mut SceneView,paths:&[String],now:i64,runtime:f32,ordinary_bounds:&[[f32;4]]){
  if self.proposals.is_empty(){return;}
  assert_eq!(paths.len(),view.files.len(),"Exact path/view index correspondence");
  if now<self.previous_time {self.terminal_started.clear();}
  for p in &self.proposals {let s=fold(p,now);if (s.kind=="merged"||s.kind=="closed")&&self.previous_time<s.event_time&&s.event_time<=now{self.terminal_started.entry((p.path.clone(),s.event_time)).or_insert(runtime);}}
  self.previous_time=now;
  let mut labels=Vec::new();let mut open=0;let mut merged=0;let mut flashed=0;let mut oldest=0;
  let mut flashed_paths=Vec::new();let mut row_reports=Vec::new();
  for p in &self.proposals {
   let s=fold(p,now);let elapsed=self.terminal_started.get(&(p.path.clone(),s.event_time)).map(|start|runtime-start).unwrap_or(999.0);let v=if self.variant=="alternative"{Alternative.map(&s,elapsed)}else{DimHighlight.map(&s,elapsed)};
   let age=(now-p.created).max(0)/86400;
   if s.kind=="open"{open+=1;oldest=oldest.max(age);}if v.visible&&s.kind=="merged"{merged+=1;}
   if let Some(index)=paths.iter().position(|x|x==&p.path){
    let f=&mut view.files[index];let pos=f.pos;let label_pos=pos+gource_core::glam::Vec2::new(5.5,-1.0);
    view.labels.retain(|l|!matches!(l.anchor,LabelAnchor::WorldTopLeft{pos:p} if p==label_pos));
    f.alpha=if v.visible{v.alpha}else{0.0};f.size*=v.scale;
    f.colour=if s.kind=="merged"{gource_core::glam::Vec3::new(0.9,0.55,1.0)}else{gource_core::glam::Vec3::new(0.55,0.7,1.0)};
    if v.visible{
     let text=format!("{} #{} {}d {}",v.glyph,p.number,age,s.verdict);
     let screen=view.camera.project(view.viewport,label_pos.extend(0.0));
     labels.push((if p.number==self.focus{2}else if s.kind=="merged"{1}else{0},age,index,label_pos,text));
     row_reports.push(format!("{{\"number\":{},\"state\":\"{}\",\"ageDays\":{},\"alpha\":{},\"generation\":{},\"actor\":\"{}\",\"owner\":\"{}\",\"x\":{},\"y\":{}}}",p.number,s.kind,age,v.alpha,s.generation,s.actor,p.owner,screen.x,screen.y));
    }
   }
  }
  // Exact associations are prepared by SHA intersection with original git changes only.
  // Look up absolute path in present FileView list; never add, touch or replay a committed file.
  for land in &self.lands {
   if land.time>now{continue;}
   let Some(p)=self.proposals.iter().find(|p|p.path==land.proposal) else {continue;};
   let s=fold(p,now);let elapsed=self.terminal_started.get(&(p.path.clone(),s.event_time)).map(|start|runtime-start).unwrap_or(999.0);if land.time!=s.event_time||!DimHighlight.map(&s,elapsed).flash{continue;}
   if let Some(i)=paths.iter().position(|p|p==&land.path){
    if flashed_paths.contains(&land.path){continue;}
    let f=&mut view.files[i];f.colour=gource_core::glam::Vec3::new(1.0,0.95,0.55);f.alpha=1.0;f.size*=1.45;
    flashed+=1;flashed_paths.push(land.path.clone());
   }
  }
  labels.sort_by(|a,b|b.0.cmp(&a.0).then(b.1.cmp(&a.1)).then(a.2.cmp(&b.2)));
  let cap=if self.variant=="uncapped"{usize::MAX}else if view.viewport.x<500.0{12}else{24};
  let visible_markers=labels.len();let mut shown_numbers=Vec::new();let mut bounds:Vec<(f32,f32,f32,f32)>=ordinary_bounds.iter().map(|b|(b[0],b[1],b[2],b[3])).collect();
  let mut focused_label=None;let mut focus_leader=false;
  for (priority,_,index,mut pos,text) in labels {
   if shown_numbers.len()>=cap{break;}
   let original=view.camera.project(view.viewport,pos.extend(0.0));
   let width=text.len() as f32*7.5;
   let valid=|b:(f32,f32,f32,f32)|b.0>=6.0&&b.1>=65.0&&b.2<=view.viewport.x-6.0&&b.3<=view.viewport.y-45.0&&!bounds.iter().any(|a|b.0<a.2+3.0&&b.2+3.0>a.0&&b.1<a.3+3.0&&b.3+3.0>a.1);
   let mut point=original;let mut b=(point.x,point.y,point.x+width,point.y+16.0);
   if self.variant!="uncapped"&&!valid(b) {
    if priority!=2{continue;}
    // Focused local evidence label only: bounded screen-distance search, not follow-PR.
    let mut found=None;
    'search:for radius in [24.0,48.0,72.0,96.0,144.0,192.0] {for (dx,dy) in [(1.0,0.0),(-1.0,0.0),(0.0,-1.0),(0.0,1.0),(1.0,-1.0),(-1.0,-1.0),(1.0,1.0),(-1.0,1.0)] {
     let x=(original.x+dx*radius).clamp(6.0,(view.viewport.x-width-6.0).max(6.0));let y=(original.y+dy*radius).clamp(65.0,(view.viewport.y-61.0).max(65.0));let candidate=(x,y,x+width,y+16.0);
     if valid(candidate){found=Some(candidate);break 'search;}
    }}
    let Some(candidate)=found else {continue;};b=candidate;point=gource_core::glam::Vec2::new(b.0,b.1);
    let matrix=view.camera.world_to_clip(view.viewport);let clip=matrix*pos.extend(0.0).extend(1.0);let world=matrix.inverse()*gource_core::glam::Vec4::new((point.x/view.viewport.x*2.0-1.0)*clip.w,(1.0-point.y/view.viewport.y*2.0)*clip.w,clip.z,clip.w);pos=(world.truncate()/world.w).truncate();
    // Existing beam pass draws a thin local label connector, not a history action.
    view.actions.push(gource_core::view::ActionView{src:view.files[index].pos,dest:pos,target_size:0.6,colour:gource_core::glam::Vec3::new(0.55,0.7,1.0),progress:0.65});focus_leader=true;
   }
   if priority==2{focused_label=Some(point);}
   bounds.push(b);shown_numbers.push(paths[index].rsplit('#').next().unwrap_or("0").to_string());
   let text=view.push_text(&text);view.labels.push(LabelView{anchor:LabelAnchor::WorldTopLeft{pos},text,font:FontRole::Small,colour:gource_core::glam::Vec4::new(0.8,0.85,1.0,0.9)});
  }
  let key=view.push_text(if view.viewport.x<500.0{"P open M merged X closed | age=d"}else{"PR prototype: P dim/open  M highlight/merged  X closed  + approved  ! changes  C comment"});
  view.hud.title=Some(key);
  if view.viewport.x<500.0 {let note=view.push_text("Reviews: + approved  ! changes  C comment");view.hud.filter_note=Some(note);}
  let focus_xy=focused_label.map(|p|format!("[{},{}]",p.x,p.y)).unwrap_or("null".into());
  self.report=format!("{{\"replayTime\":{},\"open\":{},\"oldestDays\":{},\"mergeHighlight\":{},\"flashedPresentFiles\":{},\"flashedPaths\":[{}],\"focusNumber\":{},\"focusLabel\":{},\"focusLeader\":{},\"markers\":{},\"labels\":{},\"labelCap\":{},\"shownNumbers\":[{}],\"rows\":[{}]}}",now,open,oldest,merged,flashed,flashed_paths.iter().map(|p|format!("\"{}\"",p)).collect::<Vec<_>>().join(","),self.focus,focus_xy,focus_leader,visible_markers,shown_numbers.len(),cap.min(10000),shown_numbers.join(","),row_reports.join(","));
 }
}
#[cfg(test)]mod tests{
 use super::*;
 fn p()->Proposal{Proposal{path:"/src/.pull-requests/#1".into(),number:1,created:10,owner:"owner".into(),events:vec![Event{time:10,kind:"opened".into(),actor:"owner".into()},Event{time:20,kind:"closed".into(),actor:"closer".into()},Event{time:20,kind:"reopened".into(),actor:"reopener".into()}]}}
 #[test]fn close_reopen_is_stream_ordered_and_replay_age_is_original(){let p=p();let s=fold(&p,100*86400);assert_eq!(s.kind,"open");assert_eq!(s.generation,3);assert_eq!(p.created,10);assert_eq!(p.owner,"owner");assert_eq!(s.actor,"reopener");}
 #[test]fn style_substitution_does_not_change_truth(){let s=fold(&p(),25);let before=s.clone();assert_ne!(DimHighlight.map(&s,0.1).alpha,Alternative.map(&s,0.1).alpha);assert_eq!(s,before);let mut m=s.clone();m.kind="merged".into();assert!(DimHighlight.map(&m,0.1).alpha>DimHighlight.map(&s,0.1).alpha);assert!(DimHighlight.map(&m,0.1).flash);assert!(!DimHighlight.map(&m,2.1).visible);}
 #[test]fn exact_file_flash_changes_view_only_and_never_invents_missing(){use gource_core::view::FileView;let mut probe=Probe::default();probe.load("PR\t/pr/#1\t1\t10\towner\nEV\t/pr/#1\t10\topened\towner\nEV\t/pr/#1\t20\tmerged\tmerger\nLAND\t/pr/#1\t20\t/src/landed\nLAND\t/pr/#1\t20\t/src/missing").unwrap();let mut view=SceneView::default();view.files=vec![FileView::default(),FileView::default()];let original_len=view.files.len();probe.apply(&mut view,&["/pr/#1".into(),"/src/landed".into()],21,0.1,&[]);assert_eq!(view.files.len(),original_len);assert_eq!(view.files[1].alpha,1.0);assert!(probe.report.contains("\"flashedPresentFiles\":1"));}
}
