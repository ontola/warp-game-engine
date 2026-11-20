S=${S:?set S to a scratch folder holding pv/ and a copy of games/dead-miles as dmt2}
for n in "$@"; do mkdir -p $S/pv/$n; KERN_ADAPTER=software /home/user/warp-game-engine/target/release/warp script $S/dmt2 $S/pv/$n.json --out $S/pv --players 2 --seed ${SEED:-11} 2>&1 | grep -v '"step"\|"capture"' > $S/pv/$n.log; done
