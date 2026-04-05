# Prompt — Build the Sapphire Terminal UI Correctly

Build the **real terminal user interface** for Sapphire CLI orchestration.

This is not a side decoration task.  
This is the actual operator interface for the supervisor, watchdog, worker states, and final session summary.

You must design and implement the TUI correctly.

---

## Core product truth

Sapphire is a **terminal-first orchestration layer** for terminal-native AI agents.

The UI must feel:
- real
- structured
- lightweight
- readable
- modern
- operational
- beautiful without visual noise

Do **not** build a fake web-app-in-terminal.
Do **not** add gradients, luxury garbage, oversized decoration, or noisy color spam.
Do **not** over-design it.
Do **not** make it cute.
Make it sharp, minimal, structured, and visually clean.

---

## Theme rules

### 1. Do not reinvent the background theme
The terminal background must follow the **user’s existing terminal theme**.

That means:
- use the background color the user already set in Ghostty / terminal
- do not paint over the full UI with your own fake background
- do not force a hardcoded dark background
- do not create a separate Sapphire background identity that fights the terminal

The safest and cleanest choice is:
- **respect the terminal’s existing background**
- treat background as inherited from the user’s terminal theme

This is required.

### 2. Sapphire accent theme
Sapphire should still have a visual identity through restrained accents.

Use a minimal accent palette built around:
- **bright purple** as primary accent
- **off-white / soft white / white leaning slightly gray** for main text
- **muted gray** for secondary text
- **bright green** only for success / validated / healthy state
- optional subtle cool blue hint only if it materially improves hierarchy

Do not use a rainbow palette.
Do not use high-saturation chaos.
Do not use red unless required for true error states.
Do not make every element colorful.

Color must create hierarchy, not noise.

---

## Visual design principles

### 1. Minimal but not dead
The UI must be minimal, but it must still feel alive and premium.

That means:
- excellent spacing
- clear section hierarchy
- precise borders or separators
- strong information grouping
- restrained accent usage
- excellent readability

### 2. Structure over decoration
Visual cleanliness must come from:
- layout
- typography hierarchy
- spacing
- borders
- status badges
- grouped sections
- alignment

Not from:
- gradients
- heavy boxes everywhere
- fake card shadows
- pointless icons everywhere
- excessive color variation

### 3. Operational feel
This is not a chat app.
This is not a note-taking app.
This is not a marketing page.

This is a **supervision console** for an AI agent factory.

It must feel like:
- an operator dashboard
- a senior engineer’s control surface
- a high-signal execution interface

---

## Markdown rendering requirements

Supervisor output must support **real structured markdown**.

This is mandatory.

The TUI must support and render, cleanly and reliably:

- headings
- subheadings
- bullet lists
- numbered lists
- emphasis
- bold sections
- fenced code blocks
- inline code
- block quotes if needed
- tables
- dividers / separators
- compact status sections

### Markdown rendering quality rules
Supervisor markdown must render:
- extremely clean
- extremely structured
- visually stable
- readable in narrow and wide layouts
- with strong spacing discipline

### Tables
Tables matter a lot.

Tables must render in a way that is:
- highly legible
- column-aligned
- cleanly bordered or separated
- visually compact but not cramped
- suitable for worker status / validation / risk summaries

Do not let markdown tables degrade into broken text sludge.

### Code blocks
Code blocks must:
- preserve indentation
- remain visually distinct
- use restrained styling
- not fight the user’s terminal theme
- feel clean and integrated

---

## Required UI surfaces

Build the TUI around these primary surfaces.

### 1. Supervisor view
This is the main screen.

It should show:
- mission / session title
- current orchestration state
- supervisor markdown output
- validation queue
- major blockers
- contradiction warnings
- final summary when complete

This is the primary reading surface.

### 2. Worker status area
Show a concise live status panel for workers.

Per worker, show:
- worker id / name
- role
- current state
- last short summary
- validation result if available
- success / blocked / failed / running state

Keep this concise.
Do not dump full transcript by default.

### 3. Event / watchdog status area
Show:
- what the watchdog is doing
- whether workers are healthy
- whether contradictions or stalls were detected
- whether validation is pending
- whether supervisor intervention is in progress

This should feel operational, not verbose.

### 4. Final session summary
At the end, show a clean condensed summary:
- what happened
- what succeeded
- what failed
- what testing was done
- what remains risky
- final mission state

---

## Layout rules

Use a layout that is simple, stable, and readable.

Recommended structure:
- top header strip
- main supervisor markdown pane
- side or bottom worker/watchdog status pane
- optional footer for key status/help

### Header
Header should include:
- Sapphire identity
- session / mission label
- agent type
- total workers
- overall status

Keep it clean and compact.

### Main pane
The main pane is where supervisor markdown renders.

This pane should be:
- the highest priority visual area
- optimized for reading structured markdown
- capable of rendering tables and sections beautifully

### Secondary pane
Use a secondary pane for:
- worker statuses
- watchdog summaries
- validation queue
- errors / blockers

This pane must be concise and glanceable.

---

## Typography and text hierarchy

In terminal UI, hierarchy comes from:
- color
- spacing
- weight simulation
- borders
- capitalization discipline
- line grouping

Use these carefully.

### Text roles
Define visual roles for:
- page title
- section title
- subsection title
- primary body text
- muted metadata
- success
- warning
- error
- active state
- selected / focused state

Primary text should be:
- clean off-white
- slightly softened, not harsh pure white everywhere

Muted text should be:
- gray enough to recede
- not so dim that it becomes annoying

Purple should be:
- used as the identity and structural accent
- not sprayed everywhere

Green should be:
- reserved for success / validated / healthy / completed

---

## Borders and separators

Use borders sparingly and intentionally.

Allowed:
- thin separators
- minimal bordered panels
- clean line dividers
- restrained boxes for important grouped content

Not allowed:
- everything in a box
- thick visual prison bars
- decorative border nonsense

The UI should breathe.

---

## Unicode / startup branding

When Sapphire CLI starts, render the Sapphire ASCII / Unicode title block.

Use this startup branding:

```text

  /@@@@@@   /@@@@@@  /@@@@@@@  /@@@@@@@  /@@   /@@ /@@@@@@ /@@@@@@@  /@@@@@@@@
 /@@__  @@ /@@__  @@| @@__  @@| @@__  @@| @@  | @@|_  @@_/| @@__  @@| @@_____/
| @@  \__/| @@  \ @@| @@  \ @@| @@  \ @@| @@  | @@  | @@  | @@  \ @@| @@      
|  @@@@@@ | @@@@@@@@| @@@@@@@/| @@@@@@@/| @@@@@@@@  | @@  | @@@@@@@/| @@@@@   
 \____  @@| @@__  @@| @@____/ | @@____/ | @@__  @@  | @@  | @@__  @@| @@__/   
 /@@  \ @@| @@  | @@| @@      | @@      | @@  | @@  | @@  | @@  \ @@| @@      
|  @@@@@@/| @@  | @@| @@      | @@      | @@  | @@ /@@@@@@| @@  | @@| @@@@@@@@
 \______/ |__/  |__/|__/      |__/      |__/  |__/|______/|__/  |__/|________/