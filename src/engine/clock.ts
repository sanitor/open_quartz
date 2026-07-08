export interface TimeState {
  time: number;
  delta: number;
  frame: number;
  date: Float32Array;
  fps: number;
}

const FPS_WINDOW = 60;

export class Clock {
  private startTime = 0;
  private lastTime = 0;
  private elapsed = 0;
  private frameCount = 0;
  private paused = false;
  private pauseStart = 0;
  private pauseElapsed = 0;
  private needsStart = true;

  // Pre-allocated FPS ring buffer — no allocations per frame
  private readonly fpsSamples = new Float64Array(FPS_WINDOW);
  private fpsIndex = 0;
  private fpsCount = 0;

  // Pre-allocated date vector [year, month, day, secondsOfDay]
  private readonly dateVec = new Float32Array(4);

  // Single reusable TimeState — mutated in place, never re-allocated
  private readonly state: TimeState = {
    time: 0,
    delta: 0,
    frame: 0,
    date: this.dateVec,
    fps: 0,
  };

  start(): void {
    this.startTime = 0;
    this.lastTime = 0;
    this.elapsed = 0;
    this.frameCount = 0;
    this.paused = false;
    this.pauseStart = 0;
    this.pauseElapsed = 0;
    this.needsStart = true;
    this.fpsIndex = 0;
    this.fpsCount = 0;
    this.fpsSamples.fill(0);
    this.dateVec[0] = 0;
    this.dateVec[1] = 0;
    this.dateVec[2] = 0;
    this.dateVec[3] = 0;
    this.state.time = 0;
    this.state.delta = 0;
    this.state.frame = 0;
    this.state.fps = 0;
  }

  tick(now: DOMHighResTimeStamp): TimeState {
    // First tick after start/reset: anchor times, emit frame 0 with delta 0
    if (this.needsStart) {
      this.startTime = now;
      this.lastTime = now;
      this.needsStart = false;
    }

    const s = this.state;

    if (this.paused) {
      s.delta = 0;
      // time, frame, fps stay frozen; just refresh date
      this.updateDate();
      return s;
    }

    const rawDelta = (now - this.lastTime) / 1000; // ms → s
    // Clamp to avoid spiral-of-death after tab sleep
    const delta = Math.min(rawDelta, 0.1);
    this.lastTime = now;

    this.elapsed = (now - this.startTime) / 1000 - this.pauseElapsed;

    s.time = this.elapsed;
    s.delta = delta;
    s.frame = this.frameCount++;

    // FPS ring buffer
    this.fpsSamples[this.fpsIndex] = delta;
    this.fpsIndex = (this.fpsIndex + 1) % FPS_WINDOW;
    if (this.fpsCount < FPS_WINDOW) this.fpsCount++;

    let sum = 0;
    for (let i = 0; i < this.fpsCount; i++) {
      sum += this.fpsSamples[i];
    }
    s.fps = sum > 0 ? this.fpsCount / sum : 0;

    this.updateDate();
    return s;
  }

  pause(): void {
    if (this.paused) return;
    this.paused = true;
    this.pauseStart = this.lastTime;
  }

  resume(): void {
    if (!this.paused) return;
    this.paused = false;
    // Accumulate total paused duration so elapsed stays continuous
    this.pauseElapsed += (this.lastTime - this.pauseStart) / 1000;
    // Prevent a large delta spike on the resume frame
    this.lastTime = performance.now();
  }

  reset(): void {
    this.start();
  }

  seek(t: number): void {
    this.elapsed = t;
    // Recompute startTime so subsequent ticks continue from t
    this.startTime = this.lastTime - (t + this.pauseElapsed) * 1000;
    this.state.time = t;
  }

  private updateDate(): void {
    const d = new Date();
    this.dateVec[0] = d.getFullYear();
    this.dateVec[1] = d.getMonth(); // 0-based, matches Shadertoy
    this.dateVec[2] = d.getDate();
    this.dateVec[3] =
      d.getHours() * 3600 + d.getMinutes() * 60 + d.getSeconds() + d.getMilliseconds() / 1000;
  }
}
