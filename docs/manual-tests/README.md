# Manual Image Tests

This directory contains manual image test sessions used to exercise and document
how the MCP server behaves against different image models and prompt styles.
These tests are **exploratory**, not part of automated CI.

The sessions focus on:

- Sparse vs detailed prompts (short descriptions vs rich, multi-clause prompts).
- Single-subject vs multi-subject scenes (animals, people, and composed scenes).
- Differences between GPT-family image models and Gemini-family image models.
- Composed prompts that combine multiple subjects into one scene.
- Non-photorealistic styles (abstract shapes, stylized landscapes, neon wireframes, pixel art).

## Example Outputs

The following PNGs are representative non-photorealistic outputs captured
during these sessions. Paths are relative to this `docs/manual-tests/`
directory:

![Abstract geometric shapes](images/nonreal-abstract-shapes.png)
![Pastel floating landscape](images/nonreal-pastel-landscape.png)
![Complex fantasy scene](images/nonreal-fantasy-scene.png)

Additional examples comparing different models on more realistic prompts are
available in the other PNG files under `images/`.

For additional images, see the other files in this directory.
