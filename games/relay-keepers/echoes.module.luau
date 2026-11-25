-- Echoes: the hourglass sends a keeper five seconds back in time. They
-- reappear where they stood five seconds ago, while their past self (the
-- echo) replays everything they did, holding plates and lending a head,
-- until it reaches the hourglass and vanishes into it.
return {
    {
        name="Echo Plate",
        start={{2,0},{3.5,0}},
        goal={18,0},
        land={{0,20,0}},
        hourglass={0.6,0},
        pad={6,0},
        gate=11,
    },
    {
        name="Lend a Hand",
        start={{2,0},{3.5,0}},
        goal={18.5,0},
        land={{0,20,0},{12,15,2.65}},
        hourglass={0.6,0},
        pad={5,0},
        gate=9,
        key={13.5,2.65},
        lock=17,
    },
    {
        name="On My Own Shoulders",
        start={{2.5,0},{17,0}},
        goal={18.5,0},
        land={{0,20,0},{4,7,2.65}},
        hourglass={1,0},
        pad={13,0},
        gate=10,
        key={5.5,2.65},
        lock=15,
    },
}
