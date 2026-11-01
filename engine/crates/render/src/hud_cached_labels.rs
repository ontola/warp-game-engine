//! Bounded label bitmaps for continuously scaled HUDs. Rasterize a few fixed
//! sizes; animate quads, never re-rasterize an alphabet for each float size.
use super::*;
use std::sync::Arc;
const LIMIT:usize=16*1024*1024;
#[derive(Clone,Hash,PartialEq,Eq)]
struct Key {text:String,font:FontFace,align:TextAlign,size:u32}
struct Entry {id:HudImage,picture:Arc<crate::picture::Picture>,origin:[f32;2],width:f32,used:u64}
#[derive(Default)]
pub(super) struct Labels {entries:HashMap<Key,Entry>,clock:u64,next:u64,bytes:usize}
impl Labels {
    pub fn convert(&mut self,list:&HudDrawList,fonts:&mut FontCache,swash:&mut SwashCache)->Result<Option<HudDrawList>,String> {
        // Custom strokes and colour glyphs retain the original glyph renderer.
        if !list.outlines.is_empty(){return Ok(None);}
        self.clock+=1;fonts.sync_registered();
        let mut out=HudDrawList::new(list.size);
        out.optics=list.optics.clone();out.vision_tints=list.vision_tints.clone();out.vision_background_tints=list.vision_background_tints.clone();out.pictures=list.pictures.clone();
        for layer in list.layer_ranges()? {
            out.begin_overlay();
            for (range,image) in list.image_spans(layer.vertices) {out.bind(image);out.vertices.extend_from_slice(&list.vertices[range.start as usize..range.end as usize]);}
            for run in &list.text[layer.text] {
                if !run.font_size.is_finite()||!(1.0..=512.).contains(&run.font_size)||run.text.len()>16_384 {return Err("invalid cached label".into());}
                let size=if run.font_size<=32.{32}else if run.font_size<=96.{64}else{128};
                let key=Key{text:run.text.clone(),font:run.font,align:run.align,size};
                if !self.entries.contains_key(&key) {
                    let Some((picture,origin,width))=raster(fonts,swash,&key) else{return Ok(None)};
                    // Never evict pictures referenced by the current frame.
                    while self.bytes+picture.rgba.len()>LIMIT||self.entries.len()>=256 {
                        let old=self.entries.iter().filter(|(_,v)|v.used!=self.clock).min_by_key(|(_,v)|v.used).map(|(k,_)|k.clone());
                        let Some(old)=old else{return Ok(None)};
                        self.bytes-=self.entries.remove(&old).unwrap().picture.rgba.len();
                    }
                    self.next+=1;self.bytes+=picture.rgba.len();
                    self.entries.insert(key.clone(),Entry{id:HudImage(0xfca0_0000_0000_0000+self.next),picture:Arc::new(picture),origin,width,used:self.clock});
                }
                let entry=self.entries.get_mut(&key).unwrap();entry.used=self.clock;
                let scale=run.font_size/size as f32;
                let align=match run.align {TextAlign::Left=>0.,TextAlign::Center=>0.5,TextAlign::Right=>1.};
                let x=run.position[0]+(run.width-entry.width*scale)*align+entry.origin[0]*scale;
                let y=run.position[1]+entry.origin[1]*scale;
                out.provide_image(entry.id,&entry.picture,ImageFilter::Linear);
                out.image([x,y,entry.picture.width as f32*scale,entry.picture.height as f32*scale],entry.id,[0.,0.,1.,1.],run.color.map(|c|c as f32/255.),run.bounds);
            }
        }
        Ok(Some(out))
    }
    pub fn ids(&self)->HashSet<HudImage>{self.entries.values().map(|v|v.id).collect()}
}
fn raster(fonts:&mut FontCache,swash:&mut SwashCache,key:&Key)->Option<(crate::picture::Picture,[f32;2],f32)> {
    let mut buffer=fonts.buffer(&key.text,key.font,key.size as f32,1e6,TextAlign::Left,Wrap::None);
    if buffer.layout_runs().any(|r|r.glyphs.iter().any(|g|g.glyph_id==0))&&!fonts.system_fallback {fonts.load_fallback();buffer=fonts.buffer(&key.text,key.font,key.size as f32,1e6,TextAlign::Left,Wrap::None);}
    let width=buffer.layout_runs().map(|r|r.line_w).fold(0.,f32::max);
    if key.align!=TextAlign::Left {buffer=fonts.buffer(&key.text,key.font,key.size as f32,width.max(1.),key.align,Wrap::None);}
    let mut glyphs=Vec::new();let (mut minx,mut miny,mut maxx,mut maxy)=(0i32,0i32,1i32,1i32);
    for line in buffer.layout_runs(){for glyph in line.glyphs.iter(){
        let p=glyph.physical((0.,0.),1.);let Some(image)=swash.get_image(&mut fonts.fonts,p.cache_key).as_ref() else{continue};
        if image.content!=glyphon::cosmic_text::SwashContent::Mask{return None;}
        let x=p.x+image.placement.left;let y=line.line_y.round() as i32+p.y-image.placement.top;
        minx=minx.min(x);miny=miny.min(y);maxx=maxx.max(x+image.placement.width as i32);maxy=maxy.max(y+image.placement.height as i32);
        glyphs.push((x,y,image.clone()));
    }}
    // Transparent padding preserves antialiased edges under bilinear filtering.
    minx-=1;miny-=1;maxx+=1;maxy+=1;let (w,h)=((maxx-minx) as u32,(maxy-miny) as u32);
    if w>4096||h>1024||w as usize*h as usize*4>LIMIT{return None;}
    let mut rgba=vec![0;w as usize*h as usize*4];
    for (x,y,image) in glyphs {for iy in 0..image.placement.height {for ix in 0..image.placement.width {
        let a=image.data[(iy*image.placement.width+ix) as usize] as u32;let at=(((y-miny) as u32+iy)*w+(x-minx) as u32+ix) as usize*4;
        let old=rgba[at+3] as u32;rgba[at..at+3].fill(255);rgba[at+3]=(a+old*(255-a)/255) as u8;
    }}}
    Some((crate::picture::Picture{width:w,height:h,rgba},[minx as f32,miny as f32],width))
}

#[cfg(test)]mod tests {
    use super::*;
    #[test]fn scaled_labels_reuse_pictures_and_preserve_layer_order(){
        let mut cache=Labels::default();let mut fonts=FontCache::new().unwrap();let mut swash=SwashCache::new();
        let mut list=HudDrawList::new([320,180]);list.cached_text=true;
        list.text.push(TextRun{id:1,text:"ROUND 12".into(),position:[20.,20.],bounds:[10.,10.,280.,150.],font_size:60.,color:[255;4],font:FontFace::Title,align:TextAlign::Left,width:280.});
        list.begin_overlay();list.rect([0.,0.,20.,20.],[1.,0.,0.,1.],[0.,0.,320.,180.]);
        let first=cache.convert(&list,&mut fonts,&mut swash).unwrap().unwrap();let id=first.pictures[0].0;
        let layers=first.layer_ranges().unwrap();assert_eq!(layers.len(),2);assert_eq!(layers[0].vertices,0..6);assert_eq!(layers[1].vertices,6..12);
        assert_eq!(first.vertices[0].clip,[10.,10.,280.,150.]);
        for i in 0..80 {list.text[0].font_size=60.+i as f32*0.07;list.text[0].position[0]+=0.1;list.text[0].width+=0.2;
            let next=cache.convert(&list,&mut fonts,&mut swash).unwrap().unwrap();assert_eq!(next.pictures[0].0,id);}
        assert_eq!(cache.entries.len(),1);
    }
    #[test]fn cache_evicts_old_labels_and_keeps_memory_bounded(){
        let mut cache=Labels::default();let mut fonts=FontCache::new().unwrap();let mut swash=SwashCache::new();
        let mut list=HudDrawList::new([320,180]);list.text.push(TextRun{id:1,text:String::new(),position:[0.;2],bounds:[0.,0.,320.,180.],font_size:24.,color:[255;4],font:FontFace::Body,align:TextAlign::Left,width:300.});
        for i in 0..300 {list.text[0].text=format!("Score {i}");assert!(cache.convert(&list,&mut fonts,&mut swash).unwrap().is_some());}
        assert!(cache.entries.len()<=256&&cache.bytes<=LIMIT);
    }
}
