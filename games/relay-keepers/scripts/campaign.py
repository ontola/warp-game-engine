"""Play every authored room with two virtual controllers in the actual Warp VM.

Each waypoint is (seat, x, landing height, optional jump, optional second jump).
No player positions, flags, physics or level transitions are set by the test.
"""
import argparse
import json
from pathlib import Path
import shutil
import subprocess
import tempfile
from aurora_routes import routes as aurora_routes
from beacon_routes import routes as beacon_routes
from crab_routes import routes as crab_routes
from lantern_routes import routes as lantern_routes
from saltmarsh_routes import routes as saltmarsh_routes
from portal_routes import routes as portal_routes
from water_routes import routes as water_routes

ROOT = Path(__file__).resolve().parents[1]
# Walk, jump and double-jump waypoints. A pause on arrival lets a partner move.
def w(seat, x, y=0): return [seat, x, y, 0]
def j(seat, x, y=0): return [seat, x, y, 1]
def d(seat, x, y=0): return [seat, x, y, 2]

def dock(end):
    return [1, end, 0, 3]

def launch(x1, y1, x2, y2, delay=3):
    return [1, x1, y1, 4, x2, y2, delay]

def boost(x, y, target, top):
    return [w(2,x,y), w(1,x-0.75,y), j(1,x,y+1.2), j(1,target,top)]

ROUTES = [
    boost(5.6,0,7.3,2.65)+[w(1,18),w(2,18)],
    [w(2,3.7),j(2,7.4,1.25),w(1,3.6),j(1,6.7,1.25),
     j(1,7.4,2.45),j(1,9,3.9),w(1,18),w(2,18)],
    [w(2,11.6),j(2,12.5,1.15),w(1,11.5),j(1,11.7,1.15),
     j(1,12.5,2.35),j(1,15.6,2.65)],
    [w(1,3.4),j(1,5,1.1),w(2,14.1),j(2,15,1),w(2,18),w(1,18)],
    boost(4.6,0,6,2.65)+[w(2,14.1),j(2,15,1.3),w(2,18),w(1,18)],
    [w(1,6.26),j(1,9.4),w(2,6.1),j(2,9.4),w(2,10.6),
     w(1,9.85),j(1,10.6,1.2),j(1,12,2.65),w(1,18),w(2,18)],
    boost(2.8,0,3.5,2.65)+[w(2,5.5),j(2,8),w(2,9.6),j(2,12),w(2,14),w(1,4.4,2.65),w(2,17),
     w(2,18),w(1,5.6),j(1,8),w(1,9.6),j(1,12),w(1,18)],
    [w(1,4),w(2,18),w(1,18)],
    [w(1,3.7),w(2,10.3),w(1,2.6),w(1,3.7),w(2,18),w(1,18)],
    boost(3.7,0,4.4,2.65)+[w(2,10),j(2,11.3,0.8),w(1,3.5,2.65),
     w(1,4.4,2.65),j(2,13,1),w(2,18),w(1,10),j(1,11.3,0.8),j(1,13,1),w(1,18)],
    boost(4.7,0,6,2.65)+[w(1,7.7,2.65),d(1,11,4.7),w(1,12,4.7),w(1,18),w(2,18)],
    [w(1,5.2),w(1,12,2.65),w(2,5.2),w(2,12.9,2.65),
     w(2,13.8,2.65),j(2,17),w(2,18),w(1,13.8,2.65),j(1,17.3)],
    boost(2.8,0,3.5,2.65)+[w(2,5.7),j(2,8.5),j(2,11),w(2,12.8),w(1,4.4,2.65),j(2,14.7),
     j(2,17.7),w(2,18.5),w(1,5.7),j(1,8.5),j(1,11),w(1,12.8),
     j(1,14.7),j(1,18.4)],
    boost(3.6,0,4.5,2.65)+[w(2,0.35),w(1,9.6),j(1,10.9,1),w(2,1.3),w(2,0.35),
     w(1,14,1),d(1,16,2.5),d(1,18,4.7),w(1,16.7,2.5),
     w(1,14.4,1),w(1,16.8),w(1,19),w(2,9.8),j(2,11,1),w(2,19)],
    # Signal School
    [
        w(2,5), w(1,4.25), j(1,5,1.2), j(1,6,2.65), w(1,6,2.65),
        w(2,10), w(2,12), j(2,13,1.3), w(1,18),
    ],
    # Split Deck
    [
        w(2,3), j(2,6.8,1.2), w(1,3), j(1,6,1.2), j(1,6.8,2.4),
        j(1,9,3.8), w(2,5,1.2), w(1,10.7,3.8), j(1,14,2.5), w(2,18),
    ],
    # Low Water
    [
        w(2,3), w(1,2.25), j(1,3,1.2), j(1,3.6,2.65), w(2,5.5),
        w(1,11.8), j(1,13.8,1.2), w(2,11.8), j(2,13,1.2), j(2,13.8,2.4),
        j(2,15.4,3.85), w(2,16,3.85), w(1,13,1.2), w(1,18), w(2,18.6),
    ],
    # Hangar Roof
    [
        w(2,2.5), j(2,5.8,1.2), w(1,2.6), j(1,5,1.2), j(1,5.8,2.4),
        j(1,7,3.8), w(2,11), w(1,12.3,3.8), j(1,15,4.9), w(2,3),
        j(2,3,1.2), w(1,18), w(2,18),
    ],
    # Port and Starboard
    [
        w(2,3.4), w(1,2.65), j(1,3.4,1.2), j(1,4,2.65), w(2,12),
        w(1,15.2), w(2,14.4), j(2,15.2,1.2), j(2,16,2.65), w(1,10),
        j(1,10,1.3), w(2,18.8), w(1,18),
    ],
    # Shipwrights
    [
        w(2,4), w(1,3.25), j(1,4,1.2), j(1,5,2.65), w(2,6.5),
        j(2,10), w(1,11.5,1.1), w(2,11), j(2,13.9,1.1), w(1,13.2,1.1),
        j(1,13.9,2.3), j(1,16,3.75), w(2,12,1.1), w(2,18), w(1,18.7),
    ],
    # Bell Tower
    [
        w(2,3), j(2,6.5,1.25), w(1,3), j(1,5.7,1.25), j(1,6.5,2.45),
        j(1,8,3.85), w(2,5,1.25), w(2,8,1.25), j(2,10.7,2.5), w(1,12.3,2.5),
        w(2,12.3,2.5), j(1,12.3,3.7), j(1,14.8,5.1), w(2,11,2.5), w(1,18),
        w(2,18),
    ],
    # Cabin Signal
    [
        w(2,3.5), w(1,2.75), j(1,3.5,1.2), j(1,4,2.65), w(2,11),
        j(2,12,1.2), w(2,13.6,1.2), j(2,15,2.5), w(1,18), w(2,18.6),
    ],
    # Dead End
    [
        w(1,3), j(1,4,1.15), w(2,16.7), w(1,13), w(2,12.2),
        j(2,13,1.2), j(2,14,2.65), w(1,10), w(1,2), w(2,2),
    ],
    # Overshoot
    [
        w(2,3), w(1,2.25), j(1,3,1.2), j(1,3.5,2.65), w(2,11),
        j(2,13,1.15), w(1,4.5,2.65), w(2,13.8,1.15), j(2,16.5,2.5), w(2,17,2.5),
        w(1,18),
    ],
    # Axle Stand
    [
        w(1,6.26), w(2,6), j(2,7,0.9), j(2,8,2.45), w(1,6.3),
        j(1,9.7), w(1,12), w(2,10,2.45), j(2,13,1.25), w(1,18),
    ],
    # Double Block
    [
        w(2,3.6), w(1,2.85), j(1,3.6,1.2), j(1,4,2.65), w(2,11),
        j(2,12,1.25), w(2,13,1.25), j(2,15,1.25), w(2,16.8), w(1,13),
        j(1,14,1.25), w(2,13), j(2,13.2,1.25), j(2,14,2.45), j(2,15,3.9),
        w(1,18.1), w(2,19.2),
    ],
    # Freight Return
    [
        w(1,5.76), w(2,5.5), j(2,6.5,0.9), j(2,7.5,2.45), w(1,5.8),
        j(1,9.2), w(1,12), w(2,13.8), w(1,13), j(1,13.8,1.2),
        j(1,15,2.65), w(2,11), w(2,2), w(1,2),
    ],
    # Night Dispatcher
    [
        w(2,3.5), w(1,2.75), j(1,3.5,1.2), j(1,4,2.65), w(2,6.6),
        j(2,9.6), w(2,10), w(2,11.6), w(1,5,2.65), j(2,14.8,1.3),
        w(2,15.6,1.3), j(2,17.8,2.55), w(1,6.6), j(1,10), w(1,11.6),
        j(1,14.8,1.3), w(1,18.4),
    ],
    # Weather Vane
    [
        w(2,3), w(1,2.25), j(1,3,1.2), j(1,4,2.65), w(1,4.7,2.65),
        d(1,8,4.95), w(1,8.5,4.95), w(2,11), w(1,9.8,4.95), j(1,13,2.75),
        w(2,18),
    ],
    # Nest Boxes
    [
        w(2,2), j(2,5.5,1.25), w(1,2), j(1,4.8,1.25), j(1,5.5,2.45),
        j(1,7,3.9), w(2,4,1.25), w(1,8.7,3.9), j(1,12,2.7), w(2,11),
        w(1,13.5,2.7), d(1,16,4.95), w(1,18.9), w(2,18.9),
    ],
    # Long Glide
    [
        w(2,4), w(1,3.25), j(1,4,1.2), j(1,5,2.65), w(1,6.9,2.65),
        w(2,6), d(1,12,2.65), w(2,18), w(1,18.7),
    ],
    # Perch Exchange
    [
        w(2,3.5), w(1,2.75), j(1,3.5,1.2), j(1,4,2.65), w(1,5.8,2.65),
        d(1,9,4.7), w(1,10,4.7), w(2,13), j(2,13.8,1.15), w(1,13,1.15),
        j(1,13.8,2.35), j(1,16,3.8), w(2,12.8,1.15), w(2,1), w(1,1.6),
    ],
    # Rafter Walk
    [
        w(1,2), j(1,4,1.2), j(1,6,2.5), w(2,2), j(2,4,1.2),
        j(2,6.7,2.5), w(1,7.8,2.5), d(1,10,5.4), w(2,6,2.5), w(1,15.9,5.4),
        w(2,12), j(2,13.5,1.25), w(2,14.7,1.25), j(2,17,2.65),
    ],
    # Skyhook
    [
        w(2,4.7), w(1,3.95), j(1,4.7,1.2), j(1,6,2.65), w(1,7.8,2.65),
        w(2,6), d(1,12.7,3.35), w(1,13.2,3.35), w(2,15), w(1,15.5,3.35),
        d(1,17,5.7), w(2,18.5), w(1,19.4),
    ],
    # Fledgling
    [
        w(2,3.5), w(1,2.75), j(1,3.5,1.2), j(1,4,2.65), w(2,6.8),
        w(1,9.7,2.65), d(1,13,4.7), w(2,3), w(2,16), w(1,14.7,4.7),
        j(1,17.5,2.65), w(2,1), w(1,19.4),
    ],
    # Moon Dial
    [w(1,4), w(2,13), j(2,14,1.35), w(2,16.8), w(2,18), w(1,18)],
    # Isobar
    [
        w(1,2), j(1,3.2,1.2), w(2,9), j(2,10,1), w(1,4.4,1.2),
        w(1,3.2,1.2), w(2,17), w(1,9), j(1,10,1), w(1,18),
        w(2,18),
    ],
    # Date Line
    [
        w(2,3.6), w(1,2.85), j(1,3.6,1.2), j(1,4,2.65), w(2,11),
        w(1,5.2,2.65), w(1,4,2.65), w(2,17.5), w(2,18), w(1,18),
    ],
    # Perpetual
    [
        w(1,2), w(2,8.8), w(1,0.8), w(1,2), w(2,14.2),
        w(1,0.8), w(1,2), w(2,19), w(1,19),
    ],
    # Eastbound
    [
        w(2,17), j(2,17,1.15), w(1,10), w(2,15.8,1.15), w(2,17,1.15),
        w(1,2), w(2,19), w(2,2.6),
    ],
    # Yesterday's Roof
    [
        w(2,3), w(1,2.25), j(1,3,1.2), j(1,3.5,2.65), w(2,9.6),
        j(2,11,1), w(1,4.6,2.65), w(1,3.5,2.65), w(2,17), w(1,9.6),
        j(1,11,1), w(1,15,1), j(2,15.6,1), j(2,15.1,2.2), j(2,17,2.65),
        w(2,19.4), w(1,19),
    ],
    # Epoch Relay
    [
        w(2,2.4), w(1,1.65), j(1,2.4,1.2), j(1,3,2.65), w(2,8),
        j(2,9.3,0.8), w(1,1.9,2.65), w(1,3,2.65), w(2,13.6,0.8), j(2,14.8,1.6),
        w(1,1.9,2.65), w(1,3,2.65), w(2,19.6), w(1,8), j(1,9.3,0.8),
        w(1,13.6,0.8), j(1,14.8,1.6), w(1,18.9),
    ],
    # Piston House
    [w(1,11,2.65), w(2,3), w(2,18), w(1,18.7)],
    # Kiln Return
    [
        w(1,12,2.85), w(2,5.3), j(2,8.5), w(2,11), w(1,15,1.2),
        w(2,2), w(1,2.7),
    ],
    # Glass Bell
    [
        w(2,3.4), w(1,2.65), j(1,3.4,1.2), j(1,4,2.65), w(2,5.8),
        j(2,8), j(2,10.8,1.25), w(2,11,1.25), w(2,12.8,1.25), j(2,15.6,2.55),
        w(1,5.8,2.65), j(1,9.5), w(1,10), j(1,11,1.25), w(1,13.1,1.25),
        j(1,15.2,2.55), w(1,17.9,2.55), w(1,18.6), w(2,16.5,2.55), w(1,18.6),
    ],
    # Hammerfall
    [
        w(2,3.7), w(1,2.95), j(1,3.7,1.2), j(1,4.5,2.65), w(2,10.3,3.3),
        w(2,11.8,3.3), j(2,14.5,2.35), w(2,15.6,2.35), j(2,17.5,3.65), w(1,15),
        w(2,19.5), w(1,19),
    ],
    # Glassblower
    [
        w(2,2.4), w(1,1.65), j(1,2.4,1.2), j(1,2.6,2.65), w(2,4.7),
        j(2,6.8), j(2,9.4,1.2), w(2,10.5,1.2), w(1,4.05,2.65),
        j(1,9.4,1.2), w(1,10,1.2), w(2,11.2,1.2), w(1,10.45,1.2), j(1,11.2,2.4),
        j(1,12,3.8), w(2,12.7), w(2,12.8), j(2,14.7), j(2,17.5),
        w(2,18), w(1,13.8,3.8), j(1,18.7), w(1,19),
    ],
    # Load Bearing
    [
        w(1,5.76), w(2,5.5), j(2,6.5,0.9), j(2,7.5,2.45), w(1,4),
        w(1,5.76), j(1,9.1), w(1,11.4), j(1,12.7,1.3), w(2,10),
        j(2,11.9,1.3), j(2,12.7,2.5), j(2,15,3.9), w(2,15.5,3.9), w(1,12,1.3),
        w(1,18), w(2,19),
    ],
    # Tempered
    [
        w(1,11,2.65), w(2,3.5), w(1,12.3,2.65), j(1,14.3,2.65), j(1,16.5,2.65),
        w(2,11,2.65), w(2,10,2.65), w(2,8.3), w(2,4), w(1,19.4),
    ],
    # Wind Tunnel
    [
        w(2,3.8), w(1,3.05), j(1,3.8,1.2), j(1,5,2.65), w(1,6.9,2.65),
        d(1,12,2.65), w(2,5), w(2,14.5), j(2,15,1.25), w(1,15.8,1.25),
        d(1,17.5,3.85), w(2,18.9), w(1,19.6),
    ],
    # Sandglass
    [
        w(2,3), w(1,2.25), j(1,3,1.2), j(1,3.5,2.65), w(2,10),
        w(1,4.5,2.65), w(1,3.5,2.65), w(2,16), w(1,15.2), j(1,16,1.2),
        j(1,16.5,2.65), w(2,18.5), w(2,1), w(1,1.7),
    ],
    # Turntable
    [
        w(2,3.4), w(1,2.65), j(1,3.4,1.2), j(1,4,2.65), w(2,7),
        j(2,9.5), w(2,10), j(2,11,1.25), w(2,12.9,1.25), w(1,5,2.65),
        j(2,16), w(1,10), j(1,12,1.25), w(2,10.9), j(2,11.3,1.25),
        j(2,12,2.45), j(2,14,3.85), w(2,14.8,3.85), j(2,17,2.65), w(1,18.8),
    ],
    # Two Midnights
    [
        w(2,2.3), w(1,1.55), j(1,2.3,1.2), j(1,2.8,2.65), w(2,8),
        j(2,9.3,1), w(1,1.8,2.65), w(1,2.8,2.65), w(2,13.7,1), j(2,15,2.15),
        w(1,1.8,2.65), w(1,2.8,2.65), w(2,19.6), w(1,8), j(1,9.3,1),
        w(1,13.7,1), j(1,15,2.15), w(2,19.5),
    ],
    # Vaulted Glass
    [
        w(2,3), w(1,2.25), j(1,3,1.2), j(1,4,2.65), w(1,4.8,2.65),
        d(1,9.5,4.9), w(1,10,4.9), w(2,5.7), j(2,7.8), j(2,11),
        w(1,11.8,4.9), j(1,15.5,3.5), w(2,12.7), j(2,14.7), j(2,17),
        w(1,17,3.5), w(1,19.4), w(2,18.7),
    ],
    # Borrowed Time
    [
        w(2,2.9), w(1,2.15), j(1,2.9,1.2), j(1,3.6,2.65), w(2,10),
        j(2,11.2,1), w(1,4.7,2.65), w(1,3.6,2.65), w(2,17), w(1,10),
        j(1,11.2,1), w(1,14.8,1), d(1,16,2.65), d(1,18.5,4.9), w(2,18.5),
        w(1,16,2.65), w(1,11.2,1), w(1,4), d(1,4,2.65), w(2,16.6),
        j(2,14.5,1), w(2,1), w(1,1.6),
    ],
    # Home Signal
    [
        w(2,3), w(1,2.25), j(1,3,1.2), j(1,3.5,2.65), w(2,10),
        j(2,11,1), w(1,4.6,2.65), w(1,3.5,2.65), w(2,17), w(1,10),
        j(1,11,1), w(1,15,1), d(1,17,2.65), w(2,18), w(1,16.2,2.65),
        d(1,11.5,4.65), w(2,16.6), j(2,14.5,1), w(2,10), w(1,10.2,4.65),
        d(1,6.5,3.8), w(2,2.75), w(1,1), w(1,3.5), j(2,3.5,1.2),
        j(2,3.5,2.65), w(1,4.6), d(1,4.6,2.65), j(1,6.5,3.8),
    ],
    # All Aboard
    [
        w(1,6.6), w(2,5.6), dock(1), w(1,18), w(2,18),
    ],
    # Pier Pressure
    [
        w(1,6.4), w(2,5.6), dock(1), w(2,15.8), w(1,15),
        j(1,15.8,1.2), j(1,17,2.65), w(2,18.5),
    ],
    # Mail Boat
    [
        w(1,6.9), w(2,6), dock(1), w(2,15), j(1,10.5,1.25),
        w(1,10,1.25), j(1,13), w(2,18), w(1,18),
    ],
    # Westbound
    [
        w(2,17), j(2,16,1.2), w(2,13.9), w(1,14.45), dock(1),
        w(1,2), j(1,2,1.25), w(2,5),
    ],
    # Split Landing
    [
        w(1,7), w(2,6.1), dock(1), j(1,14,1.35), w(2,16),
        w(1,15.5,1.35), j(1,17,2.7), w(2,18.7),
    ],
    # Return Ticket
    [
        w(1,7), w(2,6), dock(1), w(2,15.7), w(1,14.9),
        j(1,15.7,1.2), j(1,16.7,2.65), w(2,18.5), w(1,19), w(1,2),
        w(2,2),
    ],
    # Last Stop
    [
        w(1,6.5), w(2,5.6), dock(1), j(1,14.4,1.25), w(2,16),
        w(2,14), j(2,15.7,1.25), w(1,15,1.25), j(1,15.7,2.45), j(1,17,3.9),
        w(2,14,1.25), w(2,10), j(2,9.5,1.2), w(1,19.5),
    ],
    # Wheelhouse
    [
        w(1,6.5), w(2,2), dock(1), w(1,15.5), w(2,18),
        w(1,18),
    ],
    # Winch Loft
    [
        *boost(3.6,0,4,2.65), w(1,5.3,2.65), dock(0), w(2,10), w(1,4,2.65),
        dock(1), j(2,12,4), w(2,14.8,4), j(2,17,5.35), w(1,18.6),
    ],
    # Ferry the Key
    [
        w(1,6.6), w(2,2), dock(1), w(1,15), j(1,16,1.3),
        w(1,19), w(2,17), w(1,14.6), j(1,16,1.3), w(2,2),
        w(1,2),
    ],
    # Countershaft
    [
        w(1,10), w(2,2), j(2,3,1.25), dock(1), w(1,12,3.6),
        w(1,13.8,3.6), j(1,15.5,4.85), w(2,8), j(2,8,1.3), w(1,17.8),
    ],
    # Midspan
    [
        w(1,6.6), w(2,2), dock(1), j(1,11.7,1.25), w(1,10,1.25),
        w(2,3), dock(0), w(2,2), dock(1), j(1,13.2),
        w(1,18), w(2,18),
    ],
    # High Wire
    [
        *boost(3.7,0,4.4,2.65), w(1,5.5,2.65), dock(0), w(2,7), w(1,4.4,2.65),
        dock(1), w(2,15,3), w(1,17.8), w(2,19),
    ],
    # Across and Back
    [
        w(1,6.7), w(2,3.6), dock(1), w(1,16), w(2,15.6),
        w(1,14.9), j(1,15.6,1.2), j(1,16.5,2.6), w(1,19), w(1,3.7),
        w(2,3), j(2,3.7,1.2), j(2,3.8,2.65), w(1,2),
    ],
    # Same Beat
    [
        w(1,6.9), w(2,6), dock(1), launch(15,3.5,13,3.5), w(1,18.4),
        w(2,19),
    ],
    # Forked Crown
    [
        w(1,6.9), w(2,6), dock(1), launch(16.6,3.65,10.7,3.25),
    ],
    # Over Under
    [
        w(1,6.6), w(2,5.8), dock(1), launch(12.5,3.6,17,0), w(2,18),
        w(1,13.8,3.6), j(1,16.5,2.25),
    ],
    # Backspring
    [
        w(1,6.8), w(2,6), dock(1), launch(13,3.55,16.7,0), w(2,3.9),
        w(1,16.7), w(1,3.15), j(1,3.9,1.2), j(1,5.7,2.6), w(2,2.2),
    ],
    # High Note
    [
        w(1,7), w(2,6.2), dock(1), launch(13,4.7,16,2.75), w(1,14.8,4.7),
        j(1,17,5.85), w(2,16.7,2.75),
    ],
    # Wing and a Prayer
    [
        w(1,6.5), w(2,5.7), dock(1), launch(10.5,3.45,16.5,3.45), w(1,9.3,3.45),
        d(1,6.5,5.7), w(2,19),
    ],
    # Double Exposure
    [
        w(1,6.6), w(2,5.8), dock(1), launch(10.4,3.65,16.5,3.65), w(1,9.3,3.65),
        j(1,7,2.5), w(2,19), w(1,6,2.5), w(1,3), w(2,4),
    ],
    # Baggage Claim
    [
        w(1,7.5), w(2,3.6), dock(1), w(1,15), w(2,15.5),
        w(1,14.75), j(1,15.5,1.2), j(1,16.7,2.65), w(2,18), w(1,18.7),
    ],
    # Air Traffic
    [
        *boost(3.4,0,4,2.65), w(1,5.2,2.65), dock(0), w(2,9.5), w(1,4,2.65),
        dock(1), w(2,11,4.3), w(2,11.8,4.3), j(2,14,2.8), d(2,16.7,5.5),
        w(1,18.5),
    ],
    # The Red Eye
    [
        w(1,6.9), w(2,6), dock(1), launch(13,3.55,17,1.25), w(2,19.6),
        w(1,15,3.55), w(1,17,1.25), w(1,12), w(1,2), w(2,2.7),
    ],
    # Ground Crew
    [
        *boost(3.6,0,4.2,2.65), w(1,5.4,2.65), dock(0), w(2,7), w(1,4.2,2.65),
        dock(1), w(2,15,3), w(1,12.5), j(1,14.5,1.15), w(2,17.8),
        w(2,14), j(2,13.7,1.15), j(2,14.5,2.35), j(2,16,3.8), w(1,18.6),
        w(2,19.2),
    ],
    # Departure Board
    [
        w(1,6.8), w(2,5.9), dock(1), launch(11.5,3.45,16.5,3.45), w(1,10,3.45),
        d(1,8,5.8), w(2,19), w(1,5.6), w(1,3), w(2,4),
    ],
    # Cloud Ladder
    [
        w(1,7.3), w(2,6.4), dock(1), launch(12,4.55,17,4.55), w(2,17.8,4.55),
        j(2,18,6.15), w(1,13.4,4.55), j(1,15,3),
    ],
    # Homeward Bound
    [
        w(1,6.8), w(2,6), dock(1), launch(11.5,3.65,16.5,3.65), w(2,19),
        w(1,10.4,3.65), d(1,8,5.65), w(2,3.6), w(1,5.7), w(1,2.85),
        j(1,3.6,1.2), j(1,4,2.65), w(2,1.8), w(1,0.7),
    ],
    # Request Stop
    [
        w(1,6.6), w(2,5.6), dock(0.28), j(1,10.4,1.4), dock(1),
        w(2,16.7),
    ],
    # Postmarks
    [
        w(1,6.3), w(2,5.4), dock(0.23), j(1,9.5,1.35), dock(1),
        j(2,16.2,1.2), w(2,18.7), w(1,11), w(1,2), w(2,2),
    ],
    # Crosswind
    [
        w(1,7), w(2,6), dock(0.45), launch(10,3.5,13.5,3.5), w(1,8.7,3.5),
        w(1,5.4), w(2,17.7),
    ],
    # Semaphore Run
    [
        w(1,6.6), w(2,2), dock(0.25), j(1,9.7,1.25), w(2,2.9),
        dock(0), w(2,2), dock(1), w(1,10.5,1.25), j(1,13.8),
        w(1,16), w(2,15.6), w(1,14.85), j(1,15.6,1.2), j(1,16.7,2.65),
        w(2,18.8),
    ],
    # Sky Mail
    [
        w(1,6.7), w(2,5.8), dock(0.43), launch(9.4,3.55,14.5,3.55), w(1,8.4,3.55),
        d(1,6,5.7), w(2,17.8), w(1,3.8), w(1,2), w(2,2),
    ],
    # Transfer Window
    [
        w(1,6.9), w(2,6.2), dock(0.32), j(1,10,2), dock(1),
        w(2,17.4,2.3), w(1,11,2), j(1,14,1.15), j(1,16.5,2.3), w(2,17.3,2.3),
        w(1,16.55,2.3), j(1,17.3,3.5), j(1,16.8,4.9), w(2,19),
    ],
    # Two Tickets Home
    [
        w(1,6.7), w(2,5.8), dock(0.46), launch(10,3.6,14.8,3.6), w(2,18.7),
        w(1,9.2,3.6), d(1,6.6,5.85), w(2,3.4), w(1,4.5,2.65), w(1,4,2.65),
        w(2,2.7), w(1,1), w(1,3.4), j(2,3.4,1.2), j(2,3.6,2.65),
        w(1,1.65), w(2,0.7),
    ],
]

for chapter in (saltmarsh_routes, aurora_routes, lantern_routes, beacon_routes, water_routes, crab_routes, portal_routes):
    ROUTES.extend(chapter(w, j, d, boost, dock, launch))

# Hold still for a number of ticks (lets an echo get into place).
def wait(seat, ticks): return [seat, 0, 0, 6, ticks]
# Walk onto the hourglass; arrives when the keeper has gone back in time.
def travel(seat, x): return [seat, x, 0, 5]

# Echoes (authored last, played after the aviary).
ROUTES += [
    # Echo Plate
    [w(1,6), w(2,18), travel(1,0.6), w(1,18)],
    # Lend a Hand
    [w(1,5), w(2,12.6), travel(1,0.6), w(1,11.85), j(1,12.6,1.2), j(1,13.5,2.65),
     w(1,18.5), w(2,18)],
    # On My Own Shoulders
    [w(1,5.5), wait(1,150), travel(1,1), w(1,4.75), wait(1,30), j(1,5.5,1.2), j(1,5.5,2.65),
     w(1,8), w(2,13), w(1,18.5), w(2,18)],
]

def literal(obj):
    if isinstance(obj, list):
        return "{" + ",".join(literal(item) for item in obj) + "}"
    return str(obj)

# This temporary hook drives the production tick. State observations only.
DRIVER = """
local playTick=tick
local testRoom,step,elapsed,stable=0,1,0,0
print("Campaign "..#rooms)
tick=function(dt)
    local plan=routes[rooms[state.room].index]
    if state.room~=testRoom then
        testRoom,step,elapsed,stable=state.room,1,0,0
        print("Testing "..rooms[testRoom].name)
    end
    local input={}
    for _,r in state.crew do input[r.id]={move_x=0,held={},pressed={}} end
    if state.phase=="play" then
        local a=assert(plan[step],"route ended before both players arrived")
        local r=state.crew[a[1]]
        local p=input[r.id]
        local delta=a[2]-r.x
        local arrived=math.abs(delta)<0.015 and math.abs(r.y-a[3])<0.03 and r.on
        if a[4]==6 then
            arrived=elapsed>=a[5]
        elseif a[4]==5 then
            arrived=state.echo~=nil and state.echo.seat==a[1]
            p.move_x=if arrived then 0 else math.clamp(delta/(moveSpeed(rooms[state.room],r)*dt),-1,1)
        elseif a[4]==3 then
            arrived=if a[2]>0 and a[2]<1 then state.ferry.t>=a[2] and state.ferry.direction==1
                else math.abs(state.ferry.t-a[2])<0.0001
        else
            p.move_x=math.clamp(delta/(moveSpeed(rooms[state.room],r)*dt),-1,1)
        end
        if a[4]==9 then
            arrived=r.on and math.abs(r.x-a[5])<0.03 and math.abs(r.y-a[6])<0.03
            if arrived then p.move_x=0 end
        end
        p.held.down=a[4]==15
        -- Hold through takeoff, then release before another jump can be pressed.
        local first=a[4]==1 or a[4]==2 or a[4]==4
        local wing=a[4]==2 and elapsed>=18
        p.pressed.south=(first and elapsed==0) or (wing and elapsed==18)
        p.held.south=(first and elapsed<16) or (wing and elapsed<34)
        if a[4]==4 then
            local mate=state.crew[2]
            local partner=input[mate.id]
            partner.pressed.south=elapsed==a[7]
            partner.held.south=elapsed>=a[7] and elapsed<a[7]+16
            partner.move_x=if elapsed>=a[7] then math.clamp((a[5]-mate.x)/(moveSpeed(rooms[state.room],mate)*dt),-1,1) else 0
            arrived=arrived and mate.on and math.abs(mate.x-a[5])<0.015 and math.abs(mate.y-a[6])<0.03
        end
        if elapsed>0 and arrived then
            stable+=1
        else stable=0 end
        if stable>=2 then step+=1 elapsed=0 stable=0 else elapsed+=1 end
        if elapsed>360 then
            error(string.format("room %d step %d: target %.2f,%.2f reached %.2f,%.2f, falls %d, past %s, open %s",
                testRoom,step,a[2],a[3],r.x,r.y,state.falls,tostring(state.past),tostring(state.open)))
        end
    end
    playTick(dt,input)
    assert(state.falls==0, string.format('fall in room %d step %d',testRoom,step))
end
"""
with tempfile.TemporaryDirectory() as temp:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("warp", help="Warp executable")
    parser.add_argument("--room", type=int, help="Play just this campaign room, numbered from 1")
    args = parser.parse_args()
    game = Path(temp) / "game"
    shutil.copytree(ROOT,game,ignore=shutil.ignore_patterns("scripts"))
    src = (game / "main.luau").read_text()
    if args.room is not None:
        include = 'local rooms = require("rooms")'
        assert src.count(include) == 1
        src = src.replace(include, f'{include}\nrooms={{(assert(rooms[{args.room}],"no such campaign room"))}}')
    marker = "return {init=init,"
    assert src.count(marker) == 1
    (game/"main.luau").write_text(src.replace(marker,"local routes="+literal(ROUTES)+"\n"+DRIVER+"\n"+marker))
    ticks = 1400*len(ROUTES) if args.room is None else 362*20+100
    run = subprocess.run([args.warp,"check",str(game),"--ticks",str(ticks)],capture_output=True,text=True)
    log=run.stdout+run.stderr
    for line in log.splitlines():
        if "Testing " in line or "hook" in line or "target " in line or "route ended" in line:
            print(line)
    snapshot=json.loads(run.stdout[run.stdout.index('{\n'):])
    state=snapshot["state"]
    count=int(next(line for line in log.splitlines() if line.startswith("Campaign "))[9:])
    seen=[line for line in run.stdout.splitlines() if line.startswith("Testing ")]
    assert len(seen)==count and len(set(seen))==count, seen
    assert not snapshot["errors"] and run.returncode==0, snapshot["errors"]
    assert state["phase"]=="complete" and state["room"]==count and state["falls"]==0, state
    label = f"Room {args.room}" if args.room else f"All {count} rooms"
    print(f"{label} solved with two controllers, zero falls.")
