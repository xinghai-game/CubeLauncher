import { tick } from 'svelte';

/** Animate navigation without remounting pages or resetting form state. */
export function revealPage(node: HTMLElement, key: string) {
  let animations: Animation[] = [];
  let generation = 0;
  const preference = window.matchMedia('(prefers-reduced-motion: reduce)');
  const cancel = () => { animations.forEach((animation) => animation.cancel()); animations = []; };
  const onPreference = () => { if (preference.matches) cancel(); };
  preference.addEventListener('change', onPreference);

  async function reveal() {
    const current = ++generation;
    cancel();
    await tick();
    if (current !== generation) return;
    node.parentElement?.scrollTo({ top: 0, behavior: 'instant' });
    if (preference.matches || !node.animate) return;
    animations = Array.from(node.children).slice(0, 9).map((child, index) =>
      child.animate(
        [{ opacity: 0, transform: 'translateY(12px)' }, { opacity: 1, transform: 'translateY(0)' }],
        { duration: 420, delay: index * 35, easing: 'cubic-bezier(.22, 1, .36, 1)', fill: 'backwards' }
      )
    );
  }
  void reveal();
  return {
    update(next: string) { if (next !== key) { key = next; void reveal(); } },
    destroy() { generation++; cancel(); preference.removeEventListener('change', onPreference); }
  };
}
