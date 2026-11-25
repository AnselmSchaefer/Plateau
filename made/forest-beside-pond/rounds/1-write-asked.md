Make this area: A forest where the trees are houses for the tiny and stools for giants.

This area lies side by side with existing Plateau areas. Match every shared border below (ground height within 15 cm, water level within 5 cm, and surface material continuous) so players of every size (`tiny`, `person`, `giant`) can walk straight across with no visible or physical seam:

### West edge (`x = 0`) — meets **The heron's pond** (its east edge)

Solid ground heights every metre along `z = 0..=64` (use `edge_height(&WEST_GROUND, p.z)` to interpolate cleanly):

```rust
const WEST_GROUND: [f32; 65] = [
    1.153, 1.174, 1.167, 1.232, 1.147, 1.062, 0.989, 0.911, 0.884, 0.892, 0.926,
    0.941, 0.948, 0.909, 0.924, 0.899, 1.016, 1.071, 1.131, 1.154, 1.203, 1.223,
    1.254, 1.290, 1.380, 1.291, 1.303, 1.377, 1.294, 1.215, 1.163, 1.109, 1.063,
    1.074, 0.957, 0.894, 0.822, 0.845, 0.825, 0.825, 0.928, 0.963, 0.987, 0.966,
    0.979, 1.041, 1.046, 1.056, 1.010, 0.981, 0.975, 1.025, 0.986, 0.981, 0.960,
    1.029, 1.038, 1.063, 1.080, 1.062, 1.056, 1.041, 1.063, 1.017, 1.038,
];
```

- **Water along this edge**: none (keep dry land along this edge).
- **Surface materials along this edge**: z = 0..64 m: grass.
- **Blend smoothly**: match these heights (within 15 cm), water level (within 5 cm), and materials right along `x = 0`, fade into your interior terrain over 4–8 m, and keep walls, cliffs, or steep obstacles at least 3 m back from the seam.

Answer with the whole of `area.rs` in one ```rust block.