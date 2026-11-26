# Artwork

The orca, sea, chart and pod drawings are generated concept art that shows the direction; they'll be redrawn as
original artwork based on the Bridge mark before anything ships.

Each file is a transparent PNG whose alpha is the ink. `orca-dark.png` and `sea-band-dark.png` are the dark-theme drawings.
The sea band holds the drawing and its mirror image side by side, so it repeats without a seam. The chart and pod are
palette PNGs of the theme's ink (#0b0b0c, and #f4f4f5 for `-dark`): `chart-faint*.png` and `pod-faint*.png` behind the
sidebar pages at the boards' strength (chart 16% ink, 20% in dark; pod 10% and 14%), `chart-firstrun*` behind the first
run (16% ink, 20% in dark). `theme.test.ts` reads each file's strongest palette entry to check text contrast over the
artwork.
