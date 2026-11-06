// Shared exact predicate for the source culler and replacement renderer.
// The two arrays are existing cull bindings9/10; consumers can bind the same
// buffers under these names. Original compact owner storage is [0,owner_count).
fn source_replacement_group_selected(owner_count:u32,group:u32)->bool {
    let base=owner_count;
    if base>spatial_members_len() || spatial_members_len()-base<8u ||
        base>arrayLength(&spatial_owners) || arrayLength(&spatial_owners)-base<8u {return false;}
    if spatial_member(base)!=0x53525031u || spatial_member(base+1u)!=1u || spatial_member(base+3u)!=owner_count {return false;}
    let groups=spatial_member(base+2u);
    if group>=groups || spatial_owners[base]!=0x53525031u || spatial_owners[base+1u]==0u ||
        spatial_owners[base+1u]!=spatial_owners[base+2u] || spatial_owners[base+3u]!=groups ||
        spatial_owners[base+4u]!=1u || spatial_owners[base+5u]!=owner_count {return false;}
    if arrayLength(&spatial_owners)-base-8u<groups {return false;}
    return spatial_owners[base+8u+group]==1u;
}
fn source_replacement_owner_selected(owner_count:u32,owner:u32)->bool {
    if owner>=owner_count || owner_count>spatial_members_len() || spatial_members_len()-owner_count<8u {return false;}
    if spatial_member(owner_count)!=0x53525031u {return false;}
    let offset=spatial_member(owner_count+4u);
    if offset>spatial_members_len() || owner>=spatial_members_len()-offset {return false;}
    return source_replacement_group_selected(owner_count,spatial_member(offset+owner));
}
