# Artwork

The orca, sea, chart and pod drawings are generated concept art that shows the direction; they'll be redrawn as
original artwork based on the Bridge mark before anything ships.

Each file is a transparent PNG whose alpha is the ink. `orca-dark.png` and `sea-dark.png` are the dark-theme drawings.
`chart-faint*.png` and `pod-faint*.png` are palette PNGs of the theme's ink (#0b0b0c, and #f4f4f5 for `-dark`) baked at
the page strength: the chart at 16% (20% dark) and the pod at 10% (14% dark) of the original mask's alpha.
`theme.test.ts` reads each file's strongest palette entry to check text contrast over the artwork.
