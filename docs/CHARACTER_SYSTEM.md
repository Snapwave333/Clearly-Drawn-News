# Luma Character Direction System

## Goal

Luma is a persistent light-bulb narrator whose performance is planned from each video's teaching intent. The renderer does not contain topic-specific character logic.

## Production contract

Every storyboard beat contains:

1. A causal narration sentence.
2. A reusable visual grammar pattern.
3. One to four focal terms.
4. A topic-specific B-roll query.
5. Luma's emotion, gesture, acting goal, and optional morph.

The creative director returns this contract as validated JSON. Invalid plans are rejected before narration, media downloads, or rendering begins.

## Character invariants

Luma always remains identifiable through:

- Pear-shaped illuminated glass body at rest.
- Visible filament.
- Metal screw base.
- White gloves, expressive shoes, eyes, brows, and mouth.
- Warm yellow palette.
- Lip motion driven from narration timing.

## Data-defined morphs

A morph is not a topic enum. It is an eleven-point normalized silhouette supplied by the storyboard plan. Blender creates a shape key from those points and interpolates from the bulb to the metaphor and back.

This supports new visual metaphors without a renderer release. The director can create silhouettes for a leaf, planet, coin, shield, molecule, hourglass, cloud, book, gear, or another simple concept while character invariants remain visible.

## Visual grammar

The supported patterns are:

- Hook
- Parts
- Flow
- Split path
- Comparison
- Scale
- Timeline
- Cause and effect
- Risk
- Cycle

Patterns are compositional layouts, not topic templates. Their labels, order, media, and character performance come from the current video's plan.

## Provider boundary

The director uses an OpenAI-compatible chat endpoint configured through environment variables. NVIDIA NIM is the default endpoint and model when `NVIDIA_API_KEY` is available. Keys are read at runtime and never written into project artifacts.

## Failure behavior

- Missing director credentials: use the local fallback plan.
- Invalid beat count, narration length, focal terms, or morph geometry: reject the generated plan.
- Media provider failure: render the planned vector scene without B-roll.
- Character or render failure: do not assemble or publish.
- Upload failure: keep local media for retry.
- Confirmed upload: remove the local final video and thumbnail.
