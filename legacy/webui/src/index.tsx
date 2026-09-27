/* @refresh reload */
import { render } from "solid-js/web";
import App from "./App";
import { initTheme } from "./stores/theme";

// The palette lands before the first paint; CSS fallbacks cover a failure.
void initTheme();
render(() => <App />, document.getElementById("root") as HTMLElement);
