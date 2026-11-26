-- Split-screen aircraft instruments. All measurements use the active camera
-- or vehicle; the optic has no target selection or tracking.
local optics={}
local ivory,green,shade="#e4edd1","#c4e5a0","#111b1bb8"

local function line(x,y,w,h,color)
	hud.box(x+1,y+1,w,h,shade)
	hud.box(x,y,w,h,color or ivory)
end

local function label(value,cx,y,size,color)
	local x=cx-hud.measure(value,size)*.5
	hud.text(value,x+1,y+1,{size=size,color=shade})
	hud.text(value,x,y,{size=size,color=color or ivory})
end

local function compass(cx,y,half,s,direction,gunner)
	local degrees=math.deg(math.atan2(direction.x,-direction.z))%360
	local sweep=if gunner then 28 else 21
	local yLine=y+17*s
	local thickness=math.max(1,s)
	local first,last=math.ceil((degrees-sweep)/5),math.floor((degrees+sweep)/5)
	for i=first,last do
		local bearing=i*5
		local x=cx+(bearing-degrees)*half/sweep
		local major=i%2==0
		line(x,yLine,thickness,(if major then 7 else 4)*s,green)
		if major and math.abs(bearing-degrees)>5 then
			label(string.format("%03d",bearing%360),x,y,13*s,green)
		end
	end
	local reading=string.format("%03d",math.round(degrees)%360)
	label(reading,cx,y-2*s,17*s)
	line(cx-3*s,yLine-1,6*s,thickness)
	line(cx-.5*s,yLine,thickness,9*s)
end

local function turretDiagram(cx,y,s,relative)
	local w,h,t=43*s,25*s,math.max(1,s)
	local left,top=cx-w/2,y-h/2
	line(left,top,w,t,green); line(left,top+h-t,w,t,green)
	line(left,top,t,h,green); line(left+w-t,top,t,h,green)
	line(cx-2*s,top-2*s,4*s,3*s,green)
	local dx,dz=-math.sin(relative),-math.cos(relative)
	local span=math.min((w/2-3*s)/math.max(.001,math.abs(dx)),(h/2-3*s)/math.max(.001,math.abs(dz)))
	local length=math.max(1,math.ceil(span/(2*s)))
	for i=0,length do
		line(cx+dx*span*i/length-t/2,y+dz*span*i/length-t/2,1.6*t,1.6*t)
	end
end

function optics.draw(seat,car,id,x,y,w,h,s,objectiveRight,objectiveBottom)
	local gunner=seat.flightRole=="gunner"
	local cameraState=camera.get(id)
	local direction=if cameraState then cameraState.target-cameraState.pos else seat.mountDir
	if not direction then return end
	local cx,cy=x+w/2,y+h/2
	local half=math.min(if gunner then 145*s else 86*s,w*.17)
	local tapeY=if cx-half<objectiveRight+8*s then objectiveBottom+8*s else y+12*s
	compass(cx,tapeY,half,s,direction,gunner)
	if not gunner then return end
	local t=math.max(1,s)
	local hor=math.min(77*s,w*.105)
	local vert=math.min(48*s,h*.13)
	local gap=10*s
	-- Leave the sight center empty so small distant objects stay readable.
	for _,side in {-1,1} do
		local left=if side<0 then cx-hor else cx+gap
		local top=if side<0 then cy-vert else cy+gap
		line(left,cy,hor-gap,t)
		line(cx,top,t,vert-gap)
		line(cx+side*hor,cy-4*s,t,8*s)
		line(cx-4*s,cy+side*vert,8*s,t)
	end
	if w>=500 and h>=226 then
		local offset=math.min(w*.29,209*s)
		local vel=car.vel
		local speed=math.sqrt(vel.x*vel.x+vel.z*vel.z)*3.6
		label("GS",cx-offset,cy-19*s,12*s,green)
		label(string.format("%d KM/H",math.round(speed)),cx-offset,cy-4*s,16*s)
		label("ALT",cx+offset,cy-19*s,12*s,green)
		label(string.format("%d M",math.round(car.pos.y)),cx+offset,cy-4*s,16*s)
	end
	turretDiagram(cx,y+h*.79,s,seat.turretYaw or 0)
end

return optics
