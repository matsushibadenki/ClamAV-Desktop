# Design notes

The reference conversation supplies the app workflow. The visual concept in `home-concept.png` was generated with the ImageGen tool as design guidance, not placed into the application as a screenshot substitute.

Prompt brief: design a complete macOS ClamAV Desktop home screen with five sidebar destinations (Home, Scan, Quarantine, History, Settings), cool white and gray surfaces, native Mac typography, a teal shield, an engine readiness panel, engine/definitions/last-scan status, three scan actions, and a restrained activity panel. Preserve a clear hierarchy and avoid promotional ornament.

Implementation follows the concept's sidebar, status block, three-column metadata, scan choices, and lower activity composition. State-specific copy uses actual engine readiness. Japanese, English, and Simplified Chinese adapt naturally to width; native SF/Hiragino/PingFang are used to fit the macOS product.

QA: browser widths 320, 375, 414, 768 and 1440px; no horizontal overflow in checked viewports; all five pages and installer modal; English, Japanese, Simplified Chinese; no console warnings/errors. Native macOS UI confirmed actual scan detection, quarantine, restore, definition update, and floating/docking activity panel. Development-only screenshots are saved in this folder for the delivery preview.
