# Code archaeology: TUI routing and usage overlays

`RoutingView`, usage totals, quota pace, and the Mesh inspector are pure
presenter projection state. Core/Mesh own route selection and quota policy;
CLI obtains data; App only folds events and renders it. Keeping the types
alongside each other preserves the boundary that the terminal does not make
routing decisions or query Store.

The sensitive contract is projection equivalence: remote snapshots and the
native inspector must expose exactly the data delivered through presenter
events, including candidate ordering, selected row, conservation verdict and
quota pace. Existing TUI tests cover native Mesh rendering, remote overlay
projection, usage projection, and the long-session replay seam. This extraction
keeps all event handlers and render entry points in App and changes no data.
