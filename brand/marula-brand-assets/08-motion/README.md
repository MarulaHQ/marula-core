# Logo pop-in

`logo-popin.html` is the entry animation for splash screens, login and first load. It uses plain CSS animations, so it drops into any web project.

Timeline (about 1.9 seconds):

| Time | What happens |
|---|---|
| 0 to 750 ms | The fruit drops about 1.4 times its height, squashes to 1.14 x 0.86 on landing, rebounds to 0.94 x 1.06, then settles |
| 500 ms | The stem grows (scaleY 0 to 1, 250 ms) |
| 600 ms | The leaf springs out from its base (scale 0 to 1, rotate from -40 degrees to 0, 550 ms) |
| 850 ms | The highlight fades in to 75 percent (400 ms) |
| 1100 ms | The mark slides from the centre to its lockup position (800 ms) |
| 1250 ms onward | The letters of "marula" rise 24 to 28 px with opacity, staggered 70 ms apart |

Rules:

- Honour `prefers-reduced-motion`: skip straight to the final state. The file already does this.
- Use it on entry moments only. Never animate the logo inside the running app.
- To replay in a page, remove the animated element and insert a fresh copy (as the Replay button does), because re-adding the same element will not restart CSS animations reliably.
