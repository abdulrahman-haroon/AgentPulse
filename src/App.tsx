import { getCurrentWindow } from "@tauri-apps/api/window";
import Dashboard from "./Dashboard";
import Widget from "./Widget";

// Both windows load the same bundle; the window label picks the view.
const isWidget = getCurrentWindow().label === "widget";
if (isWidget) document.documentElement.classList.add("is-widget");

export default function App() {
  return isWidget ? <Widget /> : <Dashboard />;
}
