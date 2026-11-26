import { useLayoutEffect, useRef, useState } from "react";

// NotchAnimation.swift's MorphText holds the outgoing text through the first
// 60 ms of blur. Rapid tool updates keep the label legible between changes.
export function CompactMorphText({ text, title, className, streamsRapidly = false }: {
  text: string;
  title: string;
  className: string;
  streamsRapidly?: boolean;
}) {
  const label = useRef<HTMLSpanElement>(null);
  const [displayed, setDisplayed] = useState(text);
  const state = useRef({
    displayed: text,
    lastChange: -Infinity,
    swap: undefined as number | undefined,
    animation: undefined as Animation | undefined,
  });

  useLayoutEffect(() => {
    const current = state.current;
    return () => {
      window.clearTimeout(current.swap);
      current.animation?.cancel();
    };
  }, []);

  useLayoutEffect(() => {
    const current = state.current;
    const element = label.current;
    const pending = current.swap !== undefined;
    window.clearTimeout(current.swap);
    current.swap = undefined;
    if (text === current.displayed) {
      if (pending) current.animation?.cancel();
      return;
    }
    const update = () => {
      current.displayed = text;
      setDisplayed(text);
    };
    if (!element?.animate) {
      update();
      return;
    }
    const now = performance.now();
    const animateTo = (blur: number, duration: number) => {
      const from = getComputedStyle(element);
      const before = { filter: from.filter, opacity: from.opacity };
      current.animation?.cancel();
      current.animation = element.animate([
        before,
        { filter: `blur(${blur * 4}px)`, opacity: 1 - blur * 0.15 },
      ], { duration, easing: "ease-out", fill: "forwards" });
    };
    const blurred = Number(getComputedStyle(element).opacity) < 1 - 0.01 * 0.15;
    const streaming = streamsRapidly && (now - current.lastChange < 220 || blurred);
    current.lastChange = now;
    if (streaming) {
      update();
      if (blurred) animateTo(0, 120);
    } else {
      animateTo(1, 100);
      current.swap = window.setTimeout(() => {
        current.swap = undefined;
        update();
        animateTo(0, 150);
      }, 60);
    }
  }, [text, streamsRapidly]);

  return <span ref={label} className={className} title={title}>{displayed}</span>;
}
