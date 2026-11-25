Make this standalone composable Plateau element: A cozy carved woodland arch doorway with mossy stone surround, a paneled oak door with wrought-iron hinges and a brass knocker that swings open with F, and a warm hanging lantern just inside

## Its site

The site was measured by someone 1.7 cm tall. Everything below is in your element's **local metres**: whoever measured it is 1.7 m tall in them (the engine places your element at scale 0.010000001 and turns and moves it into place). +Y is up, +Z faces out towards whoever comes to it, +X is to the right as they look at it.

- **Round about**: wood 73%, grass 26%, stone 1%.
- **Overhead**: something solid 1.37 m up.
- **Ground**: rises 0.12 m a metre. Heights in local y on a grid, rows from local z = 0.42 to 5.53 (nearest the opening first), columns from local x = -2.55 to 2.55, 0.85 m apart (`--` where something stands in the way or there is no ground):

```text
    --     --   0.37   0.10   0.45     --     --
  0.11   0.05  -0.00  -0.08  -0.07  -0.14  -0.17
  0.19   0.10   0.10   0.04  -0.01  -0.03  -0.08
  0.42   0.32   0.24   0.16   0.17   0.10   0.03
  0.57   0.48   0.39   0.35   0.31   0.21   0.12
  0.67   0.60   0.52   0.42   0.40   0.69   0.21
  0.75   0.70   0.67   0.58   0.46   0.59   0.22
```

### The opening

The origin (0, 0, 0) is on the opening's sill: its floor, in the middle, in the plane of its rim (z = 0). The wall is at z <= 0 all round; outside is z > 0.

- **Outline**: round (87% round), 2.523 m wide (x from -1.262 to 1.261), 2.694 m high (y from 0.000 to 2.694); mean radius 1.295 m about its middle (0, 1.328).
- **Depth**: 9.30 m in along -Z; 3.66 m from floor to ceiling inside.
- **Wall**: wood; at least 0.777 m of it beyond the outline at the rim.
- **Getting in**: someone 1.7 m tall (0.51 m across) walks in through it; your element must leave them room to.

The outline itself, (x, y) at z = 0, anticlockwise from its right as seen from outside (use it as it is; go a little beyond it into the wall):

```rust
const OPENING: [(f32, f32); 48] = [
    (1.2556, 1.3285), (1.2296, 1.4903), (1.1856, 1.6461), (1.1259, 1.7948), (1.0567, 1.9385), (0.9732, 2.0752),
    (0.8767, 2.2051), (0.7694, 2.3311), (0.6430, 2.4421), (0.5034, 2.5438), (0.3463, 2.6209), (0.1769, 2.6721),
    (0.0000, 2.6935), (-0.1784, 2.6837), (-0.3525, 2.6439), (-0.5155, 2.5729), (-0.6683, 2.4859), (-0.8038, 2.3760),
    (-0.9262, 2.2547), (-1.0322, 2.1205), (-1.1187, 1.9744), (-1.1832, 1.8186), (-1.2303, 1.6581), (-1.2566, 1.4939),
    (-1.2620, 1.3285), (-1.2470, 1.1643), (-1.2128, 1.0035), (-1.1594, 0.8482), (-1.0879, 0.7004), (-0.9991, 0.5618),
    (-0.8946, 0.4339), (-0.7747, 0.3188), (-0.6412, 0.2179), (-0.4949, 0.1336), (-0.3378, 0.0677), (-0.1719, 0.0225),
    (0.0000, 0.0000), (0.1747, 0.0017), (0.3483, 0.0286), (0.5167, 0.0810), (0.6757, 0.1581), (0.8210, 0.2585),
    (0.9492, 0.3793), (1.0574, 0.5171), (1.1433, 0.6684), (1.2059, 0.8289), (1.2450, 0.9949), (1.2614, 1.1624),
];
```

It is made **for this opening** and is held to it ("Fitting an opening" in your guide): its frame meets the wall all the way round, going a little into it; a leaf, if it has one, shuts it all, and `F` opens and shuts it; and whoever could walk in through the bare opening still can.

Pictures of the site (read them with your Read tool before writing):

- `/home/anselm/Projects/plateau/make/../made/elements/oak-hollow-door/site/site-ahead.png`: the site as it is, seen from straight before it
- `/home/anselm/Projects/plateau/make/../made/elements/oak-hollow-door/site/site-aside.png`: the site as it is, seen from the right before it
- `/home/anselm/Projects/plateau/make/../made/elements/oak-hollow-door/site/site-within.png`: the site as it is, seen from within, looking out

Answer with the whole of `element.rs` in one ```rust block.