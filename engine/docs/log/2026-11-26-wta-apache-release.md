# Apache missing from the public game

The November 24 browser release `31430cd5-20261124194147` included the two-seat Apache. The November 26 browser release `56346853-20261126085033-wta-c9c4e8a5` had 88 game files. Its vehicle script built a helicopter from primitives and its manifest omitted the Apache meshes, livery and optic. Opening a fresh browser session reproduced the issue. The Apache gameplay had been developed on a branch containing `f1edc24d` and `099a12df`; main had gained the phone forest and headlight changes separately.

The restored game is based on main at `99497d35`. Its web manifest contains 97 game files. All eight files under `assets/apache` match the original licensed model package, and the three low-detail woodland assets match the November 26 release. The game uses the existing shared browser runtime. That host uses a longer baked smoke lifetime for cannon impacts than the November 24 preview.

## Checking the play link

1. Open the public game with two players. Use the teal map marker at Harbor Heliport in East Docks or the marker at West Field.
2. Board a grounded Apache with the first controller's Y button. Use Y on the second controller to take the gunner seat in the same aircraft.
3. On the first controller, raise the left stick. On the second, move the right stick and hold LT. Check that the pilot gains altitude on the left and the independent gunner optic moves and zooms on the right.

The staged browser replay ran at 960 x 540 on Mancave using WebGPU and two virtual controllers. The test positioned the players next to the heliport and observed the game state and HUD. The ordinary controller handlers boarded both players and drove the flight and optic inputs. Each view occupied 479 pixels horizontally with a two-pixel separator. The browser fetched the body, both rotors, livery and gunner HUD. The native combat regression also exercised independent firing and pilot handover.
