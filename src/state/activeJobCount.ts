type ActiveJobCountListener = (count: number) => void;

let activeJobCount = 0;
const listeners = new Set<ActiveJobCountListener>();

export function setActiveJobCount(count: number): void {
  activeJobCount = count;
  for (const listener of listeners) {
    listener(count);
  }
}

export function subscribeActiveJobCount(listener: ActiveJobCountListener): () => void {
  listeners.add(listener);
  listener(activeJobCount);
  return () => {
    listeners.delete(listener);
  };
}

export function getActiveJobCount(): number {
  return activeJobCount;
}
