struct Seat {
    // position, yaw
    pos_yaw: vec4<f32>,
    // scale, phase, tempo, hop amplitude
    params: vec4<f32>,
    // body first cluster, count; arms first cluster, count
    meshes: vec4<u32>,
    // shoulder width relative to the height, unused x3
    shape: vec4<f32>,
};
// Distances are fractions of standing height. Keep the CPU seat sampler in
// humans/standing.rs in sync for players who start among the spectators.
struct CheerJump { height:f32, compression:f32, tuck:f32, arm_swing:f32 }
fn cheer_jump(params:vec4<f32>,time:f32,mood:vec2<f32>,stride:f32)->CheerJump {
    let energy=smoothstep(0.05,0.90,mood.x);
    let strength=(params.w*(0.55+0.45*energy)+0.07*energy)*(1.-mood.y)*(1.-stride);
    let peak=0.32*strength;
    let flight=max(0.05,sqrt(8.*peak*params.x/9.81));
    let period=mix(2.0,1.14,params.z);
    let t=fract(time/period+params.y)*period-0.24;
    let arc=clamp(t/flight,0.,1.);
    let air=4.*arc*(1.-arc);
    let load=smoothstep(-0.24,-0.10,t)*(1.-smoothstep(-0.10,0.,t));
    let landing=smoothstep(0.,0.09,t-flight)*(1.-smoothstep(0.09,0.30,t-flight));
    let reach=smoothstep(-0.10,0.09,t)*(1.-smoothstep(0.28*flight,flight+0.16,t));
    return CheerJump(peak*air,strength*(0.16*load+0.21*landing),
        0.085*strength*air*air,strength*(0.60*load-2.4*reach));
}
