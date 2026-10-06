import { mount } from 'svelte';
import './styles.css';
import App from './App.svelte';

// Svelte 5 requires `mount`; the legacy `new App({ target })` constructor leaves
// effects orphaned, which renders an empty page.
const app = mount(App, { target: document.getElementById('app')! });

export default app;
