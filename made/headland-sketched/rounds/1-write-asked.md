Make this area: A headland ending in a cliff, a lighthouse on it, and smugglers' tunnels only the tiny fit.

A sketch map from above is at `/home/anselm/Projects/plateau/make/../made/headland-sketched/sketch.png` — read it with your Read tool before writing. North is at the top (z = 0), south at the bottom (z = 64), west on the left (x = 0), east on the right (x = 64). Below are the shapes the engine read from the sketch in area metres:

### Areas (metres: x west→east 0..64, z north→south 0..64)

- **brown** (land / ground): ~1726 m², centre (30.3, 49.3), bounds x=[0.0, 64.0] z=[22.0, 64.0], outline: (22.1, 22.4) → (25.2, 22.0) → (29.5, 24.4) → (33.0, 30.0) → (37.1, 31.5) → (39.8, 36.4) → (47.9, 43.0) → (56.2, 45.9) → (64.0, 46.0) → (64.0, 64.0) → (0.0, 64.0) → (0.0, 46.0) → (6.9, 42.9) → (15.6, 32.5) → (16.1, 34.0) → (18.2, 25.8)
- **brown** (land / ground): ~8 m², centre (24.0, 25.5), bounds x=[22.4, 25.6] z=[23.9, 27.1], outline: (24.6, 27.1) → (22.4, 26.1) → (23.4, 23.9) → (25.6, 24.9)
- **grey** (rock / cliff): ~213 m², centre (26.2, 23.2), bounds x=[12.9, 40.0] z=[16.0, 34.0], outline: (13.0, 34.0) → (14.4, 23.2) → (18.0, 18.0) → (21.0, 16.8) → (26.5, 16.0) → (33.0, 17.5) → (37.0, 20.6) → (39.2, 24.8) → (40.0, 30.0) → (36.2, 31.5) → (33.1, 30.1) → (31.1, 26.1) → (27.8, 23.1) → (25.2, 22.0) → (22.1, 22.4) → (18.2, 25.8) → (15.8, 32.5)
- **grey** (rock / cliff): ~54 m², centre (48.9, 14.1), bounds x=[44.0, 53.8] z=[9.5, 18.8], outline: (53.4, 16.1) → (50.9, 18.5) → (46.9, 18.4) → (44.4, 15.9) → (44.4, 12.1) → (47.8, 9.6) → (51.4, 10.0) → (53.5, 12.1)
- **grey** (rock / cliff): ~22 m², centre (9.2, 12.9), bounds x=[6.8, 12.0] z=[10.2, 15.9], outline: (10.2, 10.4) → (11.9, 13.2) → (8.6, 15.9) → (7.0, 14.2) → (7.0, 11.0)
- **blue** (water): ~1982 m², centre (33.7, 18.9), bounds x=[0.0, 64.0] z=[0.0, 46.2], outline: (0.0, 0.0) → (64.0, 0.0) → (64.0, 46.0) → (55.0, 45.6) → (46.9, 42.2) → (41.8, 37.5) → (42.8, 35.0) → (41.4, 35.4) → (42.4, 36.4) → (41.4, 37.0) → (41.0, 35.0) → (40.5, 36.0) → (37.1, 31.6) → (40.0, 29.4) → (39.2, 24.8) → (37.0, 20.6) → (33.0, 17.5) → (26.5, 16.0) → (21.0, 16.8) → (17.2, 18.8) → (13.6, 25.4) → (12.9, 32.5) → (13.0, 34.0) → (14.0, 34.1) → (12.2, 37.4) → (7.4, 42.5) → (0.0, 46.0)

### Places (orange = tiny only, purple = giants only)

- **orange (tiny only)**: centre (36.5, 26.5), radius ~2.2 m, bounds x=[34.2, 38.8] z=[24.2, 28.8]
- **purple (giants only)**: centre (49.0, 14.0), radius ~2.3 m, bounds x=[46.6, 51.4] z=[11.6, 16.4]

### Words (black labels)

- **"cliff"** at (17.6, 12.5) (box x=[15.1, 20.1] z=[11.5, 13.5])
- **"sea stack"** at (47.8, 21.8) (box x=[41.6, 53.9] z=[20.8, 22.8])
- **"lighthouse"** at (22.8, 33.3) (box x=[15.8, 29.8] z=[32.0, 34.6])
- **"smugglers' tunnels"** at (48.4, 35.8) (box x=[35.6, 61.1] z=[34.5, 37.1])

### Drawn marks (black)

- mark at (24.0, 25.5), bounds x=[21.6, 26.4] z=[23.1, 27.9]
- mark at (24.0, 25.5), bounds x=[23.4, 24.6] z=[24.9, 26.1]

Follow the sketch's layout and keep its two laws (materials within 3 m; orange places for the tiny only and purple places for giants only), while making the world natural, rich and finished at every size.

Answer with the whole of `area.rs` in one ```rust block.