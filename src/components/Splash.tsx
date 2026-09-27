import { createSignal, onMount, Show } from "solid-js";
import splashArt from "../assets/splash.jpg";

/** Minimum time the splash stays up; the art deserves a look, and it hides
 *  the first catalogue fetch. */
const MIN_MS = 1600;
/** Fade-out duration (matches `.splash.is-leaving` in main.css). */
const FADE_MS = 450;

/**
 * The start screen: shown on every launch until the app has decided what to
 * render (library or setup) and at least MIN_MS have passed, then fades out.
 * `onDone` fires once the fade has finished so first-run dialogs can wait.
 */
export function Splash(props: { ready: boolean; onDone?: () => void }) {
  const [minElapsed, setMinElapsed] = createSignal(false);
  const [leaving, setLeaving] = createSignal(false);
  const [gone, setGone] = createSignal(false);

  onMount(() => {
    window.setTimeout(() => setMinElapsed(true), MIN_MS);
  });

  const shouldLeave = () => props.ready && minElapsed();

  // A plain effect-free check: the first render where both hold starts the
  // fade; re-renders after that are no-ops.
  const maybeLeave = () => {
    if (shouldLeave() && !leaving()) {
      setLeaving(true);
      window.setTimeout(() => {
        setGone(true);
        props.onDone?.();
      }, FADE_MS);
    }
    return leaving();
  };

  return (
    <Show when={!gone()}>
      <div class={`splash${maybeLeave() ? " is-leaving" : ""}`} role="presentation" data-testid="splash">
        <img class="splash-art" src={splashArt} alt="Exorchy - the eXoDOS launcher for Omarchy" draggable={false} />
      </div>
    </Show>
  );
}
