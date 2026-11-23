"""Write a texture-free glTF mesh with indexed positions, normals and named materials."""
import json
import struct


def glb(groups, materials, generator):
    data, views, accs, mats, prims = bytearray(), [], [], [], []
    def accessor(values,fmt,component,kind,bounds=False):
        data.extend(bytes(-len(data)%4))
        offset = len(data)
        pack = struct.Struct('<'+fmt).pack
        for v in values: data.extend(pack(*v))
        views.append({'buffer':0,'byteOffset':offset,'byteLength':len(data)-offset})
        acc = {'bufferView':len(views)-1,'componentType':component,'count':len(values),'type':kind}
        if bounds: acc.update(min=list(map(min,zip(*values))),max=list(map(max,zip(*values))))
        accs.append(acc)
        return len(accs)-1
    for material in materials:
        group=groups[material['name']]
        assert group.indices
        mats.append(material)
        attrs={'POSITION':accessor(group.points,'3f',5126,'VEC3',True),
               'NORMAL':accessor(group.normals,'3f',5126,'VEC3')}
        if getattr(group,'uvs',None):
            assert len(group.uvs)==len(group.points)
            attrs['TEXCOORD_0']=accessor(group.uvs,'2f',5126,'VEC2')
        prims.append({'material':len(mats)-1,'attributes':attrs,
                      'indices':accessor([(i,) for i in group.indices],'H',5123,'SCALAR')})
    doc={'asset':{'version':'2.0','generator':generator},'scene':0,'scenes':[{'nodes':[0]}],
         'nodes':[{'mesh':0}],'meshes':[{'primitives':prims}],'buffers':[{'byteLength':len(data)}],
         'materials':mats,'bufferViews':views,'accessors':accs}
    desc=json.dumps(doc,separators=(',',':')).encode()
    desc+=b' '*(-len(desc)%4)
    data.extend(bytes(-len(data)%4))
    chunks=struct.pack('<I4s',len(desc),b'JSON')+desc+struct.pack('<I4s',len(data),b'BIN\0')+data
    return struct.pack('<4sII',b'glTF',2,12+len(chunks))+chunks
