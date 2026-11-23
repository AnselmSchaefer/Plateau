Make this area: A pond with lily pads the tiny use as stepping stones, and a heron's nest only giants can reach.

A sketch map from above is at `/home/anselm/Projects/plateau/make/../made/pond-sketched/sketch.png` — read it with your Read tool before writing. North is at the top (z = 0), south at the bottom (z = 64), west on the left (x = 0), east on the right (x = 64). Below are the shapes the engine read from the sketch in area metres:

### Areas (metres: x west→east 0..64, z north→south 0..64)

- **brown** (land / ground): ~3100 m², centre (33.0, 32.6), bounds x=[0.0, 64.0] z=[0.0, 64.0], outline: (0.0, 0.0) → (64.0, 0.0) → (64.0, 64.0) → (0.0, 64.0)
- **grey** (rock / cliff): ~63 m², centre (19.6, 20.3), bounds x=[14.0, 25.1] z=[15.6, 25.0], outline: (25.0, 18.0) → (24.4, 22.5) → (21.0, 25.0) → (16.5, 23.9) → (14.2, 21.5) → (15.0, 17.0) → (17.1, 16.0) → (21.4, 16.0)
- **blue** (water): ~861 m², centre (29.2, 31.5), bounds x=[10.9, 46.6] z=[14.5, 47.0], outline: (16.4, 43.2) → (11.0, 36.1) → (11.0, 25.9) → (13.6, 19.6) → (14.5, 18.9) → (14.0, 21.0) → (16.1, 23.6) → (21.0, 25.0) → (24.4, 22.5) → (25.1, 18.8) → (23.6, 17.0) → (19.8, 15.6) → (28.0, 14.5) → (37.0, 16.4) → (43.0, 19.9) → (46.1, 25.6) → (46.2, 33.9) → (43.2, 40.8) → (38.9, 44.5) → (31.4, 46.9) → (23.4, 46.4)

### Places (orange = tiny only, purple = giants only)

- **orange (tiny only)**: centre (20.0, 20.0), radius ~2.2 m, bounds x=[17.8, 22.2] z=[17.8, 22.2]
- **purple (giants only)**: centre (53.0, 17.0), radius ~2.4 m, bounds x=[50.6, 55.4] z=[14.6, 19.4]

### Words (black labels)

- **"mossy rock"** at (18.7, 11.3) (box x=[11.2, 26.1] z=[10.0, 12.6])
- **"heron's nest"** at (51.4, 23.0) (box x=[43.2, 59.5] z=[22.0, 24.0])
- **"lily pads"** at (28.6, 35.3) (box x=[23.2, 33.9] z=[34.0, 36.6])

Follow the sketch's layout and keep its two laws (materials within 3 m; orange places for the tiny only and purple places for giants only), while making the world natural, rich and finished at every size.

Answer with the whole of `area.rs` in one ```rust block.