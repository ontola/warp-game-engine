//! Engine debug panel built from the existing native-resolution HUD primitives.
//!
//! The host maps platform input and applies returned actions. Opening the panel
//! never pauses simulation. This module performs no GPU queries or readbacks.
use crate::hud::{FontFace, HudDrawList, TextAlign, TextRun};

pub const WINDOW_ID: u64 = 0x4442_4755_4900_0000;
const TEXT_ID: u64 = WINDOW_ID;
use crate::editor_style::{EditorButton, FG, MUTED};

#[derive(Clone, Debug, PartialEq)]
pub enum DebugValue {
    Toggle(bool),
    Number { value: f64, min: f64, max: f64, step: f64, decimals: u8, unit: String },
    /// Host-defined presets. Arrow keys wrap; Enter advances one choice.
    Choice { index:usize, labels:Vec<String> },
    /// A host-defined explicit operation, such as applying staged changes.
    Button,
}

#[derive(Clone, Debug)]
pub struct DebugControl {
    /// Stable application-defined action identity, independent of row order.
    pub id: u32,
    pub label: String,
    pub value: DebugValue,
    pub enabled: bool,
    /// None places this control at the root; otherwise references DebugMenu.id.
    pub menu: Option<u32>,
    /// Specialized controls live under the initially collapsed advanced row.
    /// Compatibility for root controls only; explicit menus take precedence.
    pub advanced: bool,
    pub help: String,
}

#[derive(Clone, Debug)]
pub struct DebugPass {
    pub label: String,
    pub ms: f64,
}

#[derive(Clone, Debug)]
pub struct DebugMenu {
    pub id: u32,
    pub parent: Option<u32>,
    pub label: String,
    pub help: String,
}

#[derive(Clone, Debug, Default)]
pub struct DebugModel {
    pub controls: Vec<DebugControl>,
    /// Stable IDs form a tree. Root menu links have parent=None. Display order
    /// is controls first, then child menus, preserving each vector's order.
    pub menus: Vec<DebugMenu>,
    /// Union of measured GPU intervals, if the producer actually computes it.
    /// Never pass the sum of potentially overlapping pass durations here.
    pub gpu_active_ms: Option<f64>,
    /// First measured GPU start to last measured end; may include gaps/idle.
    /// Excludes any work outside the producer's instrumentation scope.
    pub gpu_interval_ms: Option<f64>,
    /// Actual CPU frame-call wall time, not CPU time plus GPU duration.
    pub cpu_frame_ms: Option<f64>,
    /// Actual user + kernel execution on the main thread only; excludes waits
    /// and other worker threads. None means unsupported or unavailable.
    pub cpu_execution_ms: Option<f64>,
    /// Age of the GPU sample in rendered frames; None means unavailable.
    pub gpu_age_frames: Option<u64>,
    /// Independent leaf pass durations. Bars are not an additive breakdown.
    pub gpu_passes: Vec<DebugPass>,
    /// Host-supplied status/error, for example pending/dropped sample counts.
    pub status: Option<String>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum DebugKey { Toggle, Close, Up, Down, Left, Right, Enter, PageUp, PageDown }

#[derive(Clone, Copy, Debug, PartialEq)]
pub enum DebugAction {
    SetToggle { id: u32, value: bool },
    SetNumber { id: u32, value: f64 },
    SetChoice { id:u32, index:usize },
    Activate { id: u32 },
    ResetDefaults,
}

#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct DebugResponse {
    /// The host should suppress its normal handling of this input when true.
    pub consumed: bool,
    pub action: Option<DebugAction>,
}

#[derive(Clone, Debug, Default)]
pub struct DebugUi {
    pub open: bool,
    pub advanced_open: bool,
    selected: Option<Row>,
    cursor: [f32; 2],
    scroll: f32,
    size: [u32; 2],
    interaction: Option<Interaction>,
    window: crate::editor_window::EditorWindow,
    menu: Option<u32>,
    navigation: Vec<Navigation>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Row { Control(u32), Menu(u32), Back, Advanced, Reset }

#[derive(Clone, Copy, Debug)]
struct Navigation { menu:Option<u32>, selected:Option<Row>, scroll:f32 }

#[derive(Clone, Copy, Debug, PartialEq)]
struct NumberSpec { min: f64, max: f64, step: f64 }

#[derive(Clone, Copy, Debug)]
enum Interaction {
    Hold { id:u32, direction:i32, origin:f64, last:f64, elapsed:f64, spec:NumberSpec },
    Drag { id:u32, origin:f64, last:f64, start_x:f32, spec:NumberSpec },
}

const HOLD_DELAY: f64 = 0.35;
const HOLD_PERIOD: f64 = 0.075;
const DRAG_PIXELS_PER_STEP: f32 = 12.0;

#[derive(Clone, Copy)]
struct Layout {
    panel: [f32; 4],
    body: [f32; 4],
    footer_y: f32,
    row_h: f32,
    scale: f32,

}

fn contains(rect: [f32; 4], p: [f32; 2]) -> bool {
    EditorButton(rect).contains(p)
}
fn intersection(a: [f32; 4], b: [f32; 4]) -> [f32; 4] {
    let x=a[0].max(b[0]); let y=a[1].max(b[1]);
    [x,y,(a[0]+a[2]-x).min(b[0]+b[2]-x).max(0.0),(a[1]+a[3]-y).min(b[1]+b[3]-y).max(0.0)]
}

fn control_valid(control: &DebugControl) -> bool {
    match &control.value {
        DebugValue::Toggle(_) | DebugValue::Button => true,
        DebugValue::Choice {index,labels}=>*index<labels.len(),
        DebugValue::Number { value,min,max,step,.. } => value.is_finite() && min.is_finite()
            && max.is_finite() && step.is_finite() && min <= max && *step > 0.0,
    }
}
fn menu_path(model: &DebugModel, current: Option<u32>) -> Option<Vec<&DebugMenu>> {
    let mut result:Vec<&DebugMenu>=Vec::new();let mut next=current;
    while let Some(id)=next {
        if result.len()>=32 || result.iter().any(|m|m.id==id) {return None;}
        let menu=model.menus.iter().find(|m|m.id==id)?;
        // Ambiguous IDs must not navigate to an arbitrary settings page.
        if model.menus.iter().filter(|m|m.id==id).count()!=1 {return None;}
        result.push(menu);next=menu.parent;
    }
    result.reverse();Some(result)
}
fn visible_rows(model: &DebugModel, advanced_open: bool, current: Option<u32>) -> Vec<Row> {
    let mut rows=Vec::new();
    if current.is_some() {rows.push(Row::Back);}
    rows.extend(model.controls.iter().filter(|c|c.menu==current && (current.is_some()||!c.advanced)).map(|c|Row::Control(c.id)));
    rows.extend(model.menus.iter().filter(|m|m.parent==current&&menu_path(model,Some(m.id)).is_some()).map(|m|Row::Menu(m.id)));
    if current.is_none() && model.controls.iter().any(|c|c.menu.is_none()&&c.advanced) {
        rows.push(Row::Advanced);
        if advanced_open {rows.extend(model.controls.iter().filter(|c|c.menu.is_none()&&c.advanced).map(|c|Row::Control(c.id)));}
    }
    rows.push(Row::Reset);
    rows
}
fn number(model: &DebugModel, id: u32) -> Option<(f64,NumberSpec)> {
    let c=model.controls.iter().find(|c|c.id==id)?;
    if !c.enabled || !control_valid(c) {return None;}
    match &c.value {
        DebugValue::Number {value,min,max,step,..}=>Some((*value,NumberSpec{min:*min,max:*max,step:*step})),
        _=>None,
    }
}

impl Layout {
    fn new(size: [u32; 2], model: &DebugModel, advanced_open: bool, current: Option<u32>) -> Self {
        let [w,h]=size.map(|v|v as f32);
        let scale=(h/1080.0).clamp(0.85,2.5).min(w/600.0).min(h/720.0).max(0.01);
        let margin=16.0*scale;
        let width=(550.0*scale).min((w-2.0*margin).max(0.0));
        let header=62.0*scale;
        let footer=100.0*scale;
        let row_h=38.0*scale;
        let height=(header+footer+visible_rows(model,advanced_open,current).len() as f32*row_h)
            .min((h-2.0*margin).max(0.0));
        let body_h=(height-header-footer).max(0.0);
        let x = (w-margin-width).max(0.);
        Self { panel:[x,margin,width,height], body:[x+12.0*scale,margin+header,width-24.0*scale,body_h],
            footer_y:margin+header+body_h,row_h,scale }
    }
    fn minimum(self) -> [f32; 2] {
        [450.*self.scale,self.body[1]-self.panel[1]+100.*self.scale+2.*self.row_h]
    }
    fn placed(mut self, rect:[f32;4]) -> Self {
        let [x,y,w,h]=rect;
        let header = self.body[1]-self.panel[1];
        self.panel = [x,y,w,h];
        self.body = [x+12.*self.scale,y+header,(w-24.*self.scale).max(0.),(h-header-100.*self.scale).max(0.)];
        self.footer_y = self.body[1]+self.body[3];
        self
    }
    #[cfg(test)]
    fn grip(self) -> [f32;4] { crate::editor_window::WindowFrame {rect:self.panel,scale:self.scale}.grip().0 }

    fn row(self, index: usize, scroll: f32) -> [f32; 4] {
        [self.body[0],self.body[1]+index as f32*self.row_h-scroll,self.body[2],self.row_h]
    }
    fn buttons(self, row: [f32; 4]) -> ([f32; 4],[f32; 4]) {
        let s=self.scale; let size=30.0*s; let x=row[0]+row[2]-size-7.0*s;
        ([x-108.0*s,row[1]+4.0*s,size,size],[x,row[1]+4.0*s,size,size])
    }
    fn value(self, row: [f32;4]) -> [f32;4] {
        let (minus,plus)=self.buttons(row);
        [minus[0]+minus[2],minus[1],plus[0]-minus[0]-minus[2],minus[3]]
    }
    fn choice_buttons(self,row:[f32;4])->([f32;4],[f32;4]) {
        let (mut minus,plus)=self.buttons(row);minus[0]-=92.0*self.scale;(minus,plus)
    }
    fn choice_value(self,row:[f32;4])->[f32;4] {
        let (minus,plus)=self.choice_buttons(row);
        [minus[0]+minus[2],minus[1],plus[0]-minus[0]-minus[2],minus[3]]
    }
    fn max_scroll(self, count: usize) -> f32 { (count as f32*self.row_h-self.body[3]).max(0.0) }
}

impl DebugUi {
    /// Current host-defined submenu ID; None is the root controls page.
    pub fn current_menu(&self) -> Option<u32> { self.menu }

    /// Physical-pixel cursor position, the same coordinates as HudDrawList.
    pub fn pointer_move(&mut self, cursor: [f32; 2]) {
        if !cursor.iter().all(|v|v.is_finite()) { return; }
        self.cursor=cursor;
        self.window.move_to(cursor);
    }

    /// Cancel capture on focus loss, stage/UI replacement or platform pointer
    /// cancellation. No queued repeat or drag action survives this call.
    pub fn cancel_interaction(&mut self) { self.interaction=None; self.window.cancel(); }

    pub fn pointer_up(&mut self) -> DebugResponse {
        let captured=self.interaction.is_some() || self.window.dragging();
        self.cancel_interaction();
        DebugResponse{consumed:captured||self.open,action:None}
    }

    fn normalize(&mut self, layout: Layout, rows: &[Row]) {
        if !self.selected.is_some_and(|r|rows.contains(&r)) {
            self.selected=if self.selected.is_some_and(|r|matches!(r,Row::Control(_)))&&rows.contains(&Row::Advanced) {
                Some(Row::Advanced)
            } else {rows.first().copied()};
        }
        self.scroll=self.scroll.clamp(0.0,layout.max_scroll(rows.len()));
    }
    fn sync(&mut self, size: [u32;2], model: &DebugModel) -> (Layout,Vec<Row>) {
        if self.size!=size {self.cancel_interaction();self.size=size;}
        if !self.open {self.cancel_interaction();}
        if menu_path(model,self.menu).is_none() {
            self.menu=None;self.navigation.clear();self.selected=None;self.scroll=0.0;self.cancel_interaction();
        }
        let layout=Layout::new(size,model,self.advanced_open,self.menu);
        let rect=self.window.frame(size,layout.panel,layout.minimum(),layout.scale).rect;
        let layout=layout.placed(rect);
        let rows=visible_rows(model,self.advanced_open,self.menu);
        self.normalize(layout,&rows);
        (layout,rows)
    }
    fn reveal_selected(&mut self, layout: Layout, rows: &[Row]) {
        let index=rows.iter().position(|r|Some(*r)==self.selected).unwrap_or(0);
        let top=index as f32*layout.row_h;
        if top < self.scroll { self.scroll=top; }
        if top+layout.row_h > self.scroll+layout.body[3] { self.scroll=top+layout.row_h-layout.body[3]; }
        self.normalize(layout,rows);
    }
    fn enter_menu(&mut self, model:&DebugModel, id:u32) {
        if !model.menus.iter().any(|m|m.id==id&&m.parent==self.menu)||menu_path(model,Some(id)).is_none(){return;}
        self.cancel_interaction();
        self.navigation.push(Navigation{menu:self.menu,selected:self.selected,scroll:self.scroll});
        self.menu=Some(id);self.selected=None;self.scroll=0.0;
    }
    fn back(&mut self, model:&DebugModel) {
        let Some(id)=self.menu else{return;};
        self.cancel_interaction();
        let parent=model.menus.iter().find(|m|m.id==id).and_then(|m|m.parent);
        if let Some(saved)=self.navigation.pop().filter(|saved|saved.menu==parent) {
            self.menu=saved.menu;self.selected=saved.selected;self.scroll=saved.scroll;
        } else {
            self.menu=parent;self.selected=Some(Row::Menu(id));self.scroll=0.0;self.navigation.clear();
        }
    }
    fn action(&mut self, model: &DebugModel, direction: i32, enter: bool) -> Option<DebugAction> {
        let id=match self.selected? {
            Row::Reset=>return enter.then_some(DebugAction::ResetDefaults),
            Row::Back=>{if enter||direction<0{self.back(model);}return None;}
            Row::Menu(id)=>{if enter||direction>0{self.enter_menu(model,id);}return None;}
            Row::Advanced=>{
                self.advanced_open=if enter{!self.advanced_open}else{direction>0};
                self.cancel_interaction();return None;
            }
            Row::Control(id)=>id,
        };
        let control=model.controls.iter().find(|c|c.id==id)?;
        if !control.enabled || !control_valid(control) { return None; }
        match &control.value {
            DebugValue::Toggle(value) => {
                let next=if enter {!*value}else{direction>0};
                (next!=*value).then_some(DebugAction::SetToggle{id:control.id,value:next})
            }
            DebugValue::Number {value,min,max,step,..} if !enter => {
                let next=(*value+direction as f64*step).clamp(*min,*max);
                (next!=*value).then_some(DebugAction::SetNumber{id:control.id,value:next})
            }
            DebugValue::Choice {index,labels}=>{
                let next=if !enter&&direction<0 {if *index==0{labels.len()-1}else{*index-1}}
                    else{(*index+1)%labels.len()};
                (next!=*index).then_some(DebugAction::SetChoice{id:control.id,index:next})
            }
            DebugValue::Button if enter=>Some(DebugAction::Activate{id:control.id}),
            _=>None,
        }
    }

    /// Send pressed keys only. The host should suppress repeated Toggle keys;
    /// arrow-key repeat is useful and supported. None of these actions pauses.
    pub fn on_key(&mut self, key: DebugKey, size: [u32; 2], model: &DebugModel) -> DebugResponse {
        if key==DebugKey::Toggle {
            self.cancel_interaction();
            self.open=!self.open;
            return DebugResponse{consumed:true,action:None};
        }
        let (layout,rows)=self.sync(size,model);
        if !self.open { return DebugResponse::default(); }
        self.cancel_interaction();
        let index=rows.iter().position(|r|Some(*r)==self.selected).unwrap_or(0);
        let page=(layout.body[3]/layout.row_h).floor().max(1.0) as usize;
        let mut action=None;
        match key {
            DebugKey::Close=>if self.menu.is_some(){self.back(model);}else{self.open=false;},
            DebugKey::Up=>self.selected=Some(rows[index.saturating_sub(1)]),
            DebugKey::Down=>self.selected=Some(rows[(index+1).min(rows.len()-1)]),
            DebugKey::PageUp=>self.selected=Some(rows[index.saturating_sub(page)]),
            DebugKey::PageDown=>self.selected=Some(rows[(index+page).min(rows.len()-1)]),
            DebugKey::Left=>action=self.action(model,-1,false),
            DebugKey::Right=>action=self.action(model,1,false),
            DebugKey::Enter=>action=self.action(model,0,true),
            DebugKey::Toggle=>{},
        }
        let (layout,rows)=self.sync(size,model);
        self.reveal_selected(layout,&rows);
        DebugResponse{consumed:true,action}
    }

    /// Positive lines scroll down. The host converts platform wheel direction.
    /// Scrolling outside the panel is not consumed.
    pub fn on_scroll(&mut self, lines: f32, size: [u32; 2], model: &DebugModel) -> bool {
        let (layout,rows)=self.sync(size,model);
        if !self.open || !lines.is_finite() || !contains(layout.panel,self.cursor) { return false; }
        self.cancel_interaction();
        self.scroll=(self.scroll+lines.clamp(-100.0,100.0)*layout.row_h).clamp(0.0,layout.max_scroll(rows.len()));
        true
    }

    /// Primary-button press. +/- changes one step immediately, then tick repeats.
    /// The numeric value starts a drag; capture can continue outside the panel.
    pub fn on_pointer_down(&mut self, size: [u32; 2], model: &DebugModel) -> DebugResponse {
        self.cancel_interaction();
        let (layout,rows)=self.sync(size,model);
        if !self.open || !contains(layout.panel,self.cursor) { return DebugResponse::default(); }
        let frame=crate::editor_window::WindowFrame{rect:layout.panel,scale:layout.scale};
        match self.window.pointer(size,frame,layout.minimum(),Some(self.cursor),true,true) {
            crate::editor_window::WindowEvent::Close => { self.open=false;return DebugResponse{consumed:true,action:None}; }
            crate::editor_window::WindowEvent::Handled => return DebugResponse{consumed:true,action:None},
            _=>{}
        }
        let mut action=None;
        if contains(layout.body,self.cursor) {
            let index=((self.cursor[1]-layout.body[1]+self.scroll)/layout.row_h).floor() as usize;
            if let Some(row)=rows.get(index).copied() {
                self.selected=Some(row);
                match row {
                    Row::Advanced|Row::Reset|Row::Back|Row::Menu(_)=>action=self.action(model,0,true),
                    Row::Control(id)=>{
                        if model.controls.iter().any(|c|c.id==id&&matches!(&c.value,DebugValue::Choice{..})) {
                            let rect=layout.row(index,self.scroll);let (previous,next)=layout.choice_buttons(rect);
                            if contains(previous,self.cursor){action=self.action(model,-1,false);}
                            else if contains(next,self.cursor){action=self.action(model,1,false);}
                            else if contains(layout.choice_value(rect),self.cursor){action=self.action(model,0,true);}
                        } else if let Some((value,spec))=number(model,id) {
                            let rect=layout.row(index,self.scroll);
                            let (minus,plus)=layout.buttons(rect);
                            let direction=if contains(minus,self.cursor){-1}else if contains(plus,self.cursor){1}else{0};
                            if direction!=0 {
                                let next=(value+direction as f64*spec.step).clamp(spec.min,spec.max);
                                action=(next!=value).then_some(DebugAction::SetNumber{id,value:next});
                                self.interaction=Some(Interaction::Hold{id,direction,origin:value,last:next,elapsed:0.0,spec});
                            } else if contains(layout.value(rect),self.cursor) {
                                self.interaction=Some(Interaction::Drag{id,origin:value,last:value,start_x:self.cursor[0],spec});
                            }
                        } else {action=self.action(model,0,true);}
                    }
                }
            }
        }
        DebugResponse{consumed:true,action}
    }

    /// Called once per UI frame with elapsed seconds, including after pointer
    /// movement. Returns at most one absolute value; repeat counts depend only
    /// on accumulated dt, not frame rate. Hosts apply the action immediately.
    pub fn tick(&mut self, dt_seconds: f64, size: [u32;2], model: &DebugModel) -> DebugResponse {
        let (layout,rows)=self.sync(size,model);
        if !dt_seconds.is_finite() || dt_seconds<0.0 {self.cancel_interaction();return DebugResponse::default();}
        let Some(mut interaction)=self.interaction.take() else {return DebugResponse::default();};
        let (id,stored_spec)=match interaction {
            Interaction::Hold{id,spec,..}|Interaction::Drag{id,spec,..}=>(id,spec),
        };
        let Some((_,spec))=number(model,id) else {return DebugResponse{consumed:true,action:None};};
        let Some(index)=rows.iter().position(|r|*r==Row::Control(id)) else {return DebugResponse{consumed:true,action:None};};
        if spec!=stored_spec {return DebugResponse{consumed:true,action:None};}
        let mut action=None;
        match &mut interaction {
            Interaction::Hold{direction,origin,last,elapsed,..}=>{
                let (minus,plus)=layout.buttons(layout.row(index,self.scroll));
                let button=if *direction<0{minus}else{plus};
                if !contains(intersection(button,layout.body),self.cursor) {return DebugResponse{consumed:true,action:None};}
                *elapsed+=dt_seconds;
                if !elapsed.is_finite() {return DebugResponse{consumed:true,action:None};}
                let repeats=if *elapsed+1e-9>=HOLD_DELAY {1.0+((*elapsed-HOLD_DELAY+1e-9)/HOLD_PERIOD).floor()}else{0.0};
                let next=(*origin+*direction as f64*(1.0+repeats)*spec.step).clamp(spec.min,spec.max);
                if next!=*last {action=Some(DebugAction::SetNumber{id,value:next});*last=next;}
            }
            Interaction::Drag{origin,last,start_x,..}=>{
                let steps=((self.cursor[0]-*start_x)/(DRAG_PIXELS_PER_STEP*layout.scale)).round() as f64;
                let next=(*origin+steps*spec.step).clamp(spec.min,spec.max);
                if next!=*last {action=Some(DebugAction::SetNumber{id,value:next});*last=next;}
            }
        }
        self.interaction=Some(interaction);
        DebugResponse{consumed:true,action}
    }

    /// Append to the caller's draw list. Closed UI appends nothing. Stable text
    /// IDs use a reserved namespace; callers should append this panel once.
    pub fn draw(&mut self, list: &mut HudDrawList, model: &DebugModel) {
        let (layout,rows)=self.sync(list.size,model);
        if !self.open || list.size.contains(&0) {return;}
        let first_text=list.text.len();
        let s=layout.scale; let p=layout.panel; let clip=p;
        crate::editor_window::WindowFrame { rect:p, scale:s }.draw(list,WINDOW_ID,10,"Graphics settings");
        let x=p[0]+16.0*s; let w=p[2]-32.0*s; let mut y=p[1]+5.0*s;
        y+=29.0*s;
        let path=menu_path(model,self.menu).unwrap_or_default();
        let caption=if path.is_empty(){"Live controls".into()}else{
            let names:Vec<_>=path.iter().rev().take(2).rev().map(|m|m.label.as_str()).collect();
            format!("{}{}",if path.len()>2{"... / "}else{""},names.join(" / "))
        };
        text(list,22,&caption,[x,y,w,28.0*s],clip,22.0*s,FG,TextAlign::Left);
        let first=(self.scroll/layout.row_h).floor() as usize;
        let last=((self.scroll+layout.body[3])/layout.row_h).ceil() as usize;
        for i in first..=last.min(rows.len()-1) {
            let row=layout.row(i,self.scroll); let row_clip=intersection(row,layout.body);
            if row_clip[3]<=0.0 {continue;}
            let selected=Some(rows[i])==self.selected;
            let face = EditorButton([row[0], row[1]+s, row[2], row[3]-2.*s]);
            let button = |list: &mut HudDrawList, id, label: &str, enabled| {
                face.draw(list,TEXT_ID+id,label,row_clip,22.*s,selected || contains(row,self.cursor),enabled);
            };
            match rows[i] {
                Row::Reset => { button(list,604,if self.menu.is_some() {"Reset all defaults"} else {"Reset defaults"},true); continue; }
                Row::Back => { button(list,608,"< Back",true); continue; }
                Row::Advanced => { button(list,600,if self.advanced_open {"- Advanced rendering"} else {"+ Advanced rendering"},true); continue; }
                Row::Menu(id) => {
                    if let Some(menu)=model.menus.iter().find(|m| m.id==id) {
                        button(list,(1u64<<36)+id as u64*4,&format!("{} >",menu.label),true);
                    }
                    continue;
                }
                Row::Control(_) => {}
            }
            let Row::Control(control_id)=rows[i] else {continue};
            let Some(control)=model.controls.iter().find(|c|c.id==control_id) else {continue;};
            let id=4096+control.id as u64*4;
            let enabled=control.enabled&&control_valid(control);
            let color=if enabled{FG}else{MUTED};
            if matches!(&control.value,DebugValue::Button) {
                button(list,id,&control.label,enabled);
                continue;
            }
            list.rect(face.0,if selected{[0.075,0.15,0.19,0.98]}else{[0.034,0.048,0.065,0.8]},row_clip);
            let reserved=if matches!(&control.value,DebugValue::Choice{..}){262.0}else{170.0};
            text(list,id,&control.label,[row[0]+10.0*s,row[1]+5.0*s,row[2]-reserved*s,29.0*s],row_clip,23.0*s,color,TextAlign::Left);
            match &control.value {
                DebugValue::Toggle(value)=>{
                    let button=[row[0]+row[2]-105.0*s,row[1]+5.0*s,95.0*s,28.0*s];
                    EditorButton(button).draw(list,TEXT_ID+id+1,if *value{"On"}else{"Off"},row_clip,22.*s,*value || contains(button,self.cursor),enabled);
                }
                DebugValue::Number {value,decimals,unit,..}=>{
                    let (minus,plus)=layout.buttons(row);
                    for (offset,rect,label) in [(1,minus,"-"),(2,plus,"+")] {
                        EditorButton(rect).draw(list,TEXT_ID+id+offset,label,row_clip,22.*s,contains(rect,self.cursor),enabled);
                    }
                    let shown=if control_valid(control){format!("{:.*}{}",(*decimals).min(6) as usize,value,unit)}else{"invalid".into()};
                    text(list,id+3,&shown,[minus[0]+minus[2],row[1]+6.0*s,plus[0]-minus[0]-minus[2],28.0*s],row_clip,21.0*s,color,TextAlign::Center);
                }
                DebugValue::Choice {index,labels}=>{
                    let (previous,next)=layout.choice_buttons(row);
                    for (offset,rect,label) in [(1,previous,"<"),(2,next,">")] {
                        EditorButton(rect).draw(list,TEXT_ID+id+offset,label,row_clip,22.*s,contains(rect,self.cursor),enabled);
                    }
                    text(list,id+3,labels.get(*index).map_or("invalid",String::as_str),layout.choice_value(row),row_clip,21.0*s,color,TextAlign::Center);
                }
                DebugValue::Button=>{},
            }
        }
        if layout.max_scroll(rows.len())>0.0 && layout.body[3]>0.0 {
            let total=rows.len() as f32*layout.row_h;
            let height=(layout.body[3]*layout.body[3]/total).max(8.0*s).min(layout.body[3]);
            let offset=self.scroll/layout.max_scroll(rows.len())*(layout.body[3]-height);
            list.rect([p[0]+p[2]-6.0*s,layout.body[1]+offset,3.0*s,height],[0.25,0.50,0.52,0.9],clip);
        }
        let footer=[x,layout.footer_y,w,p[1]+p[3]-layout.footer_y];
        let footer_clip=intersection(footer,clip);
        list.line([x,layout.footer_y+2.0*s],[x+w,layout.footer_y+2.0*s],1.0*s,[0.2,0.28,0.33,0.9],footer_clip);
        let help=match self.selected {
            Some(Row::Control(id))=>model.controls.iter().find(|c|c.id==id).map_or("",|c|c.help.as_str()),
            Some(Row::Menu(id))=>model.menus.iter().find(|m|m.id==id).map_or("",|m|m.help.as_str()),
            Some(Row::Back)=>"Return to the previous settings menu. Esc also goes back.",
            Some(Row::Advanced)=>"Show or hide specialized rendering controls.",
            _=>"Restore all of the host's default render settings.",
        };
        for (i,line) in [help,model.status.as_deref().unwrap_or("CPU excludes other threads; wall includes waiting."),
                        "F10 close | Esc back | Arrows | Enter open/toggle", "Drag values | Hold +/- | Wheel scroll"].into_iter().enumerate() {
            text(list,500+i as u64,line,[x,layout.footer_y+(8.0+i as f32*21.0)*s,w,22.0*s],footer_clip,17.5*s,if i==0{FG}else{MUTED},TextAlign::Left);
        }
        crate::editor_style::style_text(list, first_text);
    }
}

fn text(list: &mut HudDrawList, id: u64, value: &str, rect: [f32; 4], clip: [f32; 4], size: f32, color: [u8; 4], align: TextAlign) {
    let bounds=intersection(rect,clip);
    if bounds[2]<=0.0 || bounds[3]<=0.0 {return;}
    list.text.push(TextRun{id:TEXT_ID+id,text:value.chars().take(240).collect(),position:[rect[0],rect[1]],bounds,
        font_size:size.clamp(1.0,512.0),color,font:FontFace::Body,align,width:rect[2].max(0.0)});
}

#[cfg(test)]
mod tests {
    use super::*;
    fn model(count: usize) -> DebugModel {
        DebugModel {controls:(0..count).map(|i|DebugControl{id:100+i as u32,label:format!("Control {i}"),
            value:if i==1{DebugValue::Number{value:0.75,min:0.25,max:1.0,step:0.25,decimals:2,unit:String::new()}}else{DebugValue::Toggle(true)},
            enabled:true,menu:None,advanced:false,help:"Changes the next rendered frame.".into()}).collect(),menus:Vec::new(),
            gpu_passes:vec![DebugPass{label:"shade".into(),ms:2.5},DebugPass{label:"raster".into(),ms:1.2}],
            gpu_active_ms:Some(3.0),gpu_interval_ms:Some(4.5),cpu_frame_ms:Some(2.0),cpu_execution_ms:Some(0.75),gpu_age_frames:Some(2),status:None }
    }
    #[test]
    fn floating_window_keeps_controls_attached_and_stays_reachable() {
        let size=[1280,800];
        let model=model(8);
        let mut ui=DebugUi {open:true,..Default::default()};
        let first=ui.sync(size,&model).0;
        assert!(first.panel[0]>size[0] as f32*0.5);
        let start=[first.panel[0]+36.,first.panel[1]+12.];
        ui.pointer_move(start);
        assert!(ui.on_pointer_down(size,&model).consumed);
        ui.pointer_move([start[0]-220.,start[1]+40.]);
        ui.pointer_up();
        let moved=ui.sync(size,&model).0;
        assert!((moved.body[0]-(first.body[0]-220.)).abs()<0.01);
        let grip=moved.grip();
        let corner=[grip[0]+6.,grip[1]+6.];
        ui.pointer_move(corner);
        ui.on_pointer_down(size,&model);
        ui.pointer_move([corner[0]+100.,corner[1]-70.]);
        ui.pointer_up();
        let resized=ui.sync(size,&model).0;
        assert!((resized.panel[2]-moved.panel[2]-100.).abs()<0.01);
        assert!(resized.body[3]<moved.body[3]);
        let row=resized.row(0,0.);
        ui.pointer_move([row[0]+8.,row[1]+8.]);
        assert_eq!(ui.on_pointer_down(size,&model).action,Some(DebugAction::SetToggle{id:100,value:false}));
        ui.on_key(DebugKey::Toggle,size,&model);
        ui.on_key(DebugKey::Toggle,size,&model);
        assert_eq!(resized.panel,ui.sync(size,&model).0.panel);
        let small=ui.sync([480,320],&model).0;
        assert!(small.panel[0]>=0. && small.panel[1]>=0. && small.panel[0]+small.panel[2]<=480.01 && small.panel[1]+small.panel[3]<=320.01);
    }

    #[test]
    fn closed_panel_does_not_draw_or_consume_game_controls() {
        let mut ui=DebugUi::default();let m=model(2);let mut list=HudDrawList::new([1280,720]);
        ui.draw(&mut list,&m);assert!(list.vertices.is_empty()&&list.text.is_empty());
        assert!(!ui.on_key(DebugKey::Right,list.size,&m).consumed);
        assert!(!ui.on_pointer_down(list.size,&m).consumed);
        assert!(ui.on_key(DebugKey::Toggle,list.size,&m).consumed);assert!(ui.open);
        ui.on_key(DebugKey::Close,list.size,&m);assert!(!ui.open);
    }
    #[test]
    fn keyboard_uses_stable_ids_clamps_numbers_and_emits_reset() {
        let mut ui=DebugUi{open:true,..Default::default()};let mut m=model(2);let size=[1280,720];
        assert_eq!(ui.on_key(DebugKey::Enter,size,&m).action,Some(DebugAction::SetToggle{id:100,value:false}));
        ui.on_key(DebugKey::Down,size,&m);
        assert_eq!(ui.on_key(DebugKey::Right,size,&m).action,Some(DebugAction::SetNumber{id:101,value:1.0}));
        if let DebugValue::Number{value,..}=&mut m.controls[1].value{*value=1.0;}
        assert_eq!(ui.on_key(DebugKey::Right,size,&m).action,None);
        assert_eq!(ui.on_key(DebugKey::Left,size,&m).action,Some(DebugAction::SetNumber{id:101,value:0.75}));
        ui.on_key(DebugKey::Down,size,&m);
        assert_eq!(ui.on_key(DebugKey::Enter,size,&m).action,Some(DebugAction::ResetDefaults));
    }
    #[test]
    fn pointer_hit_testing_matches_scaled_rendered_buttons() {
        for size in [[1280,720],[1920,1080],[3840,2160]] {
            let mut ui=DebugUi{open:true,..Default::default()};let m=model(2);let layout=Layout::new(size,&m,false,None);
            let row=layout.row(0,0.0);ui.pointer_move([row[0]+10.0,row[1]+row[3]*0.5]);
            assert_eq!(ui.on_pointer_down(size,&m).action,Some(DebugAction::SetToggle{id:100,value:false}));
            let (minus,plus)=layout.buttons(layout.row(1,0.0));
            ui.pointer_move([plus[0]+plus[2]*0.5,plus[1]+plus[3]*0.5]);
            assert_eq!(ui.on_pointer_down(size,&m).action,Some(DebugAction::SetNumber{id:101,value:1.0}));
            ui.pointer_move([minus[0]+minus[2]*0.5,minus[1]+minus[3]*0.5]);
            assert_eq!(ui.on_pointer_down(size,&m).action,Some(DebugAction::SetNumber{id:101,value:0.5}));
            ui.pointer_move([size[0] as f32-1.0,20.0]);assert!(!ui.on_pointer_down(size,&m).consumed);
        }
    }
    #[test]
    fn scrolling_keeps_selection_visible_and_footer_unclickable_as_control() {
        let m=model(80);let size=[1280,720];let layout=Layout::new(size,&m,false,None);
        let mut ui=DebugUi{open:true,..Default::default()};
        for _ in 0..80{ui.on_key(DebugKey::Down,size,&m);}
        assert_eq!(ui.selected,Some(Row::Reset));
        let row=layout.row(80,ui.scroll);
        assert!(row[1]>=layout.body[1]-0.001 && row[1]+row[3]<=layout.body[1]+layout.body[3]+0.001);
        ui.pointer_move([layout.body[0]+20.0,layout.footer_y+10.0]);
        assert_eq!(ui.on_pointer_down(size,&m).action,None);
        ui.pointer_move([layout.body[0]+20.0,layout.body[1]+20.0]);
        assert!(ui.on_scroll(-100.0,size,&m));assert_eq!(ui.scroll,0.0);
        ui.pointer_move([layout.body[0]+20.0,layout.body[1]+layout.row_h*0.5]);
        assert_eq!(ui.on_pointer_down(size,&m).action,Some(DebugAction::SetToggle{id:100,value:false}));
    }
    #[test]
    fn disabled_or_invalid_controls_never_emit_changes() {
        let mut ui=DebugUi{open:true,..Default::default()};let mut m=model(2);let size=[1280,720];
        m.controls[0].enabled=false;assert_eq!(ui.on_key(DebugKey::Enter,size,&m).action,None);
        ui.on_key(DebugKey::Down,size,&m);
        if let DebugValue::Number{step,..}=&mut m.controls[1].value{*step=f64::NAN;}
        assert_eq!(ui.on_key(DebugKey::Right,size,&m).action,None);
        let mut list=HudDrawList::new(size);ui.draw(&mut list,&m);assert!(list.text.iter().any(|t|t.text=="invalid"));
    }
    #[test]
    fn graphics_settings_are_bounded_and_leave_timings_to_the_profiler() {
        for size in [[1280,720],[1920,1080],[3840,2160]] {
            let mut ui=DebugUi{open:true,..Default::default()};let mut m=model(80);
            m.gpu_passes.push(DebugPass{label:"bad".into(),ms:f64::NAN});
            let mut list=HudDrawList::new(size);ui.draw(&mut list,&m);
            let ids:std::collections::HashSet<_>=list.text.iter().map(|t|t.id).collect();
            assert_eq!(ids.len(),list.text.len());assert!(list.text.len()<100&&list.vertices.len()<1000);
            assert!(list.vertices.iter().all(|v|v.position.iter().chain(v.clip.iter()).all(|f|f.is_finite())));
            for t in &list.text {
                assert!(t.bounds[0]>=0.0&&t.bounds[1]>=0.0);
                assert!(t.bounds[0]+t.bounds[2]<=size[0] as f32+0.01);
                assert!(t.bounds[1]+t.bounds[3]<=size[1] as f32+0.01);
            }
            assert!(list.text.iter().any(|t|t.text.contains("Graphics settings")));
            assert!(!list.text.iter().any(|t|t.text=="CPU main thread"));
        }
    }
    #[test]
    fn timing_reset_never_moves_control_hit_regions_or_text() {
        for size in [[1280,720],[3840,2160]] {
            let mut ui=DebugUi{open:true,..Default::default()};let mut m=model(3);
            let mut positions=None;
            for valid in [false,true,false] {
                m.gpu_passes=if valid {(0..20).map(|i|DebugPass{label:format!("pass {i}"),ms:i as f64}).collect()}else{vec![]};
                m.gpu_interval_ms=valid.then_some(8.0);
                m.cpu_execution_ms=valid.then_some(1.25);
                let mut draw=HudDrawList::new(size);ui.draw(&mut draw,&m);
                let current:Vec<_>=draw.text.iter().filter(|t|t.id>=TEXT_ID+4096).map(|t|(t.id,t.position,t.bounds)).collect();
                if let Some(previous)=&positions {assert_eq!(&current,previous);}else{positions=Some(current);}
                let layout=Layout::new(size,&m,false,None);let (_,plus)=layout.buttons(layout.row(1,0.0));
                ui.pointer_move([plus[0]+plus[2]*0.5,plus[1]+plus[3]*0.5]);
                assert_eq!(ui.on_pointer_down(size,&m).action,Some(DebugAction::SetNumber{id:101,value:1.0}));
                ui.pointer_up();
            }
        }
    }
    fn interaction_model() -> DebugModel {
        let mut m=model(2);
        m.controls[1].value=DebugValue::Number{value:10.0,min:0.0,max:20.0,step:1.0,decimals:0,unit:String::new()};m
    }
    fn click_number(ui:&mut DebugUi,size:[u32;2],m:&DebugModel,plus:bool)->DebugResponse {
        let layout=Layout::new(size,m,ui.advanced_open,ui.menu);let (minus,right)=layout.buttons(layout.row(1,0.0));
        let rect=if plus{right}else{minus};ui.pointer_move([rect[0]+rect[2]*0.5,rect[1]+rect[3]*0.5]);
        ui.on_pointer_down(size,m)
    }
    #[test]
    fn hold_repeat_uses_accumulated_time_and_stops_after_release_or_cancel() {
        let size=[1920,1080];let m=interaction_model();
        let mut a=DebugUi{open:true,..Default::default()};let mut b=a.clone();
        assert_eq!(click_number(&mut a,size,&m,true).action,Some(DebugAction::SetNumber{id:101,value:11.0}));
        click_number(&mut b,size,&m,true);
        assert_eq!(a.tick(0.34,size,&m).action,None);
        assert_eq!(a.tick(0.01,size,&m).action,Some(DebugAction::SetNumber{id:101,value:12.0}));
        assert_eq!(a.tick(0.15,size,&m).action,Some(DebugAction::SetNumber{id:101,value:14.0}));
        assert_eq!(b.tick(0.5,size,&m).action,Some(DebugAction::SetNumber{id:101,value:14.0}));
        a.pointer_up();assert_eq!(a.tick(2.0,size,&m).action,None);
        b.cancel_interaction();assert_eq!(b.tick(2.0,size,&m).action,None);
        click_number(&mut a,size,&m,false);a.on_key(DebugKey::Close,size,&m);
        assert_eq!(a.tick(2.0,size,&m).action,None);
        a.open=true;click_number(&mut a,size,&m,true);
        assert_eq!(a.tick(0.5,[3840,2160],&m).action,None);assert!(a.interaction.is_none());
        click_number(&mut b,size,&m,true);b.pointer_move([1900.0,1000.0]);
        assert_eq!(b.tick(0.5,size,&m).action,None);assert!(b.interaction.is_none());
    }
    #[test]
    fn drag_is_resolution_independent_quantized_bounded_and_releasable() {
        for size in [[1280,720],[1920,1080],[3840,2160]] {
            let m=interaction_model();let mut ui=DebugUi{open:true,..Default::default()};
            let layout=Layout::new(size,&m,false,None);let value=layout.value(layout.row(1,0.0));
            let start=[value[0]+value[2]*0.5,value[1]+value[3]*0.5];ui.pointer_move(start);
            assert_eq!(ui.on_pointer_down(size,&m).action,None);
            ui.pointer_move([start[0]+DRAG_PIXELS_PER_STEP*layout.scale*3.0,start[1]]);
            assert_eq!(ui.tick(0.016,size,&m).action,Some(DebugAction::SetNumber{id:101,value:13.0}));
            assert_eq!(ui.tick(0.016,size,&m).action,None);
            ui.pointer_move([start[0]+10000.0,start[1]]);
            assert_eq!(ui.tick(0.016,size,&m).action,Some(DebugAction::SetNumber{id:101,value:20.0}));
            ui.pointer_move([start[0]-10000.0,start[1]]);
            assert_eq!(ui.tick(0.016,size,&m).action,Some(DebugAction::SetNumber{id:101,value:0.0}));
            ui.pointer_up();ui.pointer_move(start);assert_eq!(ui.tick(0.1,size,&m).action,None);
        }
    }
    #[test]
    fn collapsed_advanced_rows_keep_control_identity_and_keyboard_navigation() {
        let size=[1920,1080];let mut m=model(4);m.controls[0].advanced=true;m.controls[2].advanced=true;
        let mut ui=DebugUi{open:true,..Default::default()};let mut draw=HudDrawList::new(size);ui.draw(&mut draw,&m);
        assert!(!ui.advanced_open);assert_eq!(ui.selected,Some(Row::Control(101)));
        assert!(!draw.text.iter().any(|t|t.text=="Control 0"));
        assert_eq!(visible_rows(&m,false,None),[Row::Control(101),Row::Control(103),Row::Advanced,Row::Reset]);
        ui.on_key(DebugKey::Down,size,&m);ui.on_key(DebugKey::Down,size,&m);
        assert_eq!(ui.selected,Some(Row::Advanced));ui.on_key(DebugKey::Enter,size,&m);assert!(ui.advanced_open);
        ui.on_key(DebugKey::Down,size,&m);
        assert_eq!(ui.on_key(DebugKey::Enter,size,&m).action,Some(DebugAction::SetToggle{id:100,value:false}));
        let layout=Layout::new(size,&m,true,None);let header=layout.row(2,ui.scroll);
        ui.pointer_move([header[0]+20.0,header[1]+header[3]*0.5]);
        assert_eq!(ui.on_pointer_down(size,&m).action,None);assert!(!ui.advanced_open);
        assert_eq!(ui.selected,Some(Row::Advanced));ui.on_key(DebugKey::Down,size,&m);
        assert_eq!(ui.on_key(DebugKey::Enter,size,&m).action,Some(DebugAction::ResetDefaults));
    }
    fn nested_model() -> DebugModel {
        let mut m=interaction_model();
        m.menus=vec![
            DebugMenu{id:1,parent:None,label:"Category".into(),help:"Choose a group.".into()},
            DebugMenu{id:2,parent:Some(1),label:"Child A".into(),help:"First group.".into()},
            DebugMenu{id:3,parent:Some(1),label:"Child B".into(),help:"Second group.".into()},
            DebugMenu{id:4,parent:None,label:"Specialized".into(),help:"Separate group.".into()},
        ];
        m.controls[1].menu=Some(2);
        m.controls.push(DebugControl{id:500,label:"Apply changes".into(),value:DebugValue::Button,
            enabled:true,menu:Some(3),advanced:false,help:"Explicitly apply staged values.".into()});
        m
    }
    #[test]
    fn nested_navigation_hides_siblings_preserves_ids_and_escape_returns_one_level() {
        let size=[1920,1080];let mut m=nested_model();let mut ui=DebugUi{open:true,..Default::default()};
        assert_eq!(visible_rows(&m,false,None),[Row::Control(100),Row::Menu(1),Row::Menu(4),Row::Reset]);
        ui.on_key(DebugKey::Down,size,&m);ui.on_key(DebugKey::Enter,size,&m);
        assert_eq!(ui.current_menu(),Some(1));assert_eq!(ui.selected,Some(Row::Back));
        assert_eq!(visible_rows(&m,false,Some(1)),[Row::Back,Row::Menu(2),Row::Menu(3),Row::Reset]);
        ui.on_key(DebugKey::Down,size,&m);ui.on_key(DebugKey::Right,size,&m);
        assert_eq!(ui.current_menu(),Some(2));ui.on_key(DebugKey::Down,size,&m);
        assert_eq!(ui.on_key(DebugKey::Right,size,&m).action,Some(DebugAction::SetNumber{id:101,value:11.0}));
        m.controls.swap(0,1);
        let mut draw=HudDrawList::new(size);ui.draw(&mut draw,&m);
        assert_eq!(ui.selected,Some(Row::Control(101)));
        assert!(draw.text.iter().any(|t|t.text=="Category / Child A"));
        assert!(!draw.text.iter().any(|t|t.text=="Apply changes"||t.text=="Control 0"));
        ui.on_key(DebugKey::Close,size,&m);assert!(ui.open);assert_eq!(ui.current_menu(),Some(1));
        assert_eq!(ui.selected,Some(Row::Menu(2)));
        ui.on_key(DebugKey::Close,size,&m);assert!(ui.open);assert_eq!(ui.current_menu(),None);
        assert_eq!(ui.selected,Some(Row::Menu(1)));
        ui.on_key(DebugKey::Close,size,&m);assert!(!ui.open);
    }
    #[test]
    fn nested_numeric_drag_release_and_navigation_cancel_are_real_hit_regions() {
        for size in [[1280,720],[3840,2160]] {
            let m=nested_model();let mut ui=DebugUi{open:true,..Default::default()};
            ui.on_key(DebugKey::Down,size,&m);ui.on_key(DebugKey::Enter,size,&m);
            let menu_layout=Layout::new(size,&m,false,Some(1));let link=menu_layout.row(1,0.0);
            ui.pointer_move([link[0]+20.0,link[1]+link[3]*0.5]);ui.on_pointer_down(size,&m);
            assert_eq!(ui.current_menu(),Some(2));
            let layout=Layout::new(size,&m,false,Some(2));let rect=layout.value(layout.row(1,0.0));
            let start=[rect[0]+rect[2]*0.5,rect[1]+rect[3]*0.5];ui.pointer_move(start);ui.on_pointer_down(size,&m);
            ui.pointer_move([start[0]+2.0*DRAG_PIXELS_PER_STEP*layout.scale,start[1]]);
            assert_eq!(ui.tick(0.1,size,&m).action,Some(DebugAction::SetNumber{id:101,value:12.0}));
            ui.pointer_up();ui.pointer_move([start[0]+500.0,start[1]]);assert_eq!(ui.tick(1.0,size,&m).action,None);
            let (_,plus)=layout.buttons(layout.row(1,0.0));ui.pointer_move([plus[0]+5.0,plus[1]+5.0]);ui.on_pointer_down(size,&m);
            ui.on_key(DebugKey::Close,size,&m);assert_eq!(ui.current_menu(),Some(1));
            assert_eq!(ui.tick(1.0,size,&m).action,None);
        }
    }
    #[test]
    fn explicit_buttons_respect_disabled_state_and_never_activate_on_arrows() {
        let size=[1920,1080];let mut m=nested_model();let mut ui=DebugUi{open:true,..Default::default()};
        ui.enter_menu(&m,1);ui.enter_menu(&m,3);ui.on_key(DebugKey::Down,size,&m);
        assert_eq!(ui.selected,Some(Row::Control(500)));
        assert_eq!(ui.on_key(DebugKey::Right,size,&m).action,None);
        assert_eq!(ui.on_key(DebugKey::Enter,size,&m).action,Some(DebugAction::Activate{id:500}));
        m.controls.iter_mut().find(|c|c.id==500).unwrap().enabled=false;
        assert_eq!(ui.on_key(DebugKey::Enter,size,&m).action,None);
        let layout=Layout::new(size,&m,false,Some(3));let button=layout.row(1,0.0);
        ui.pointer_move([button[0]+20.0,button[1]+button[3]*0.5]);assert_eq!(ui.on_pointer_down(size,&m).action,None);
        m.controls.iter_mut().find(|c|c.id==500).unwrap().enabled=true;
        assert_eq!(ui.on_pointer_down(size,&m).action,Some(DebugAction::Activate{id:500}));
        ui.on_key(DebugKey::Down,size,&m);
        assert_eq!(ui.on_key(DebugKey::Enter,size,&m).action,Some(DebugAction::ResetDefaults));
    }
    #[test]
    fn navigation_restores_scroll_and_invalidated_menu_cancels_capture() {
        let size=[1280,720];let mut m=model(30);m.menus=nested_model().menus;
        let mut ui=DebugUi{open:true,..Default::default()};
        for _ in 0..30 {ui.on_key(DebugKey::Down,size,&m);}
        assert_eq!(ui.selected,Some(Row::Menu(1)));assert!(ui.scroll>0.0);let scroll=ui.scroll;
        ui.on_key(DebugKey::Enter,size,&m);assert_eq!(ui.scroll,0.0);
        ui.on_key(DebugKey::Close,size,&m);assert_eq!(ui.scroll,scroll);assert_eq!(ui.selected,Some(Row::Menu(1)));
        let mut m=nested_model();ui.enter_menu(&m,1);ui.enter_menu(&m,2);
        let layout=Layout::new(size,&m,false,Some(2));let rect=layout.value(layout.row(1,0.0));
        ui.pointer_move([rect[0]+5.0,rect[1]+5.0]);ui.on_pointer_down(size,&m);assert!(ui.interaction.is_some());
        m.menus.retain(|m|m.id!=1);
        assert_eq!(ui.tick(1.0,size,&m).action,None);assert_eq!(ui.current_menu(),None);assert!(ui.interaction.is_none());
        let mut m=nested_model();m.menus[0].parent=Some(2);
        assert!(menu_path(&m,Some(2)).is_none());assert!(!visible_rows(&m,false,None).contains(&Row::Menu(1)));
        let mut m=nested_model();m.menus.push(m.menus[0].clone());
        assert!(!visible_rows(&m,false,None).contains(&Row::Menu(1)));
    }
    #[test]
    fn choice_presets_wrap_and_pointer_arrows_do_not_start_numeric_capture() {
        for size in [[1280,720],[3840,2160]] {
            let mut m=nested_model();m.controls[1].value=DebugValue::Choice {index:0,labels:vec!["Morning".into(),"Noon".into(),"Night".into()]};
            let mut ui=DebugUi{open:true,..Default::default()};
            ui.enter_menu(&m,1);ui.enter_menu(&m,2);ui.on_key(DebugKey::Down,size,&m);
            assert_eq!(ui.on_key(DebugKey::Left,size,&m).action,Some(DebugAction::SetChoice{id:101,index:2}));
            if let DebugValue::Choice{index,..}=&mut m.controls[1].value{*index=2;}
            assert_eq!(ui.on_key(DebugKey::Right,size,&m).action,Some(DebugAction::SetChoice{id:101,index:0}));
            assert_eq!(ui.on_key(DebugKey::Enter,size,&m).action,Some(DebugAction::SetChoice{id:101,index:0}));
            let layout=Layout::new(size,&m,false,Some(2));let (previous,_)=layout.choice_buttons(layout.row(1,0.0));
            ui.pointer_move([previous[0]+previous[2]*0.5,previous[1]+previous[3]*0.5]);
            assert_eq!(ui.on_pointer_down(size,&m).action,Some(DebugAction::SetChoice{id:101,index:1}));
            assert!(ui.interaction.is_none());assert_eq!(ui.tick(3.0,size,&m).action,None);
            m.controls[1].enabled=false;assert_eq!(ui.on_pointer_down(size,&m).action,None);
            m.controls[1].enabled=true;m.controls[1].value=DebugValue::Choice{index:0,labels:Vec::new()};
            assert_eq!(ui.on_key(DebugKey::Enter,size,&m).action,None);
            let mut draw=HudDrawList::new(size);ui.draw(&mut draw,&m);
            assert!(draw.text.iter().any(|t|t.text=="invalid"));
            m.controls[1].value=DebugValue::Choice{index:9,labels:vec!["Only".into()]};
            assert_eq!(ui.on_key(DebugKey::Left,size,&m).action,None);
            m.controls[1].value=DebugValue::Choice{index:0,labels:vec!["Only".into()]};
            assert_eq!(ui.on_key(DebugKey::Enter,size,&m).action,None);
        }
    }
}
