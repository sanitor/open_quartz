import * as THREE from 'three';

export type VideoSourceType = 'camera' | 'file';

export interface VideoSourceConfig {
  type: VideoSourceType;
  deviceId?: string;
  facingMode?: 'user' | 'environment';
  url?: string;
  loop?: boolean;
  playbackRate?: number;
}

export class VideoSource {
  private readonly video: HTMLVideoElement;
  private readonly config: VideoSourceConfig;
  private texture: THREE.VideoTexture | null = null;
  private stream: MediaStream | null = null;
  private ready = false;

  constructor(config: VideoSourceConfig) {
    this.config = config;
    this.video = document.createElement('video');
    this.video.muted = true;
    this.video.playsInline = true;
    this.video.autoplay = true;
    this.video.loop = config.loop ?? true;
    this.video.playbackRate = config.playbackRate ?? 1;
  }

  async init(): Promise<void> {
    if (this.config.type === 'camera') {
      const video: MediaTrackConstraints = {};
      if (this.config.deviceId) video.deviceId = { exact: this.config.deviceId };
      if (this.config.facingMode) video.facingMode = this.config.facingMode;
      this.stream = await navigator.mediaDevices.getUserMedia({ video: Object.keys(video).length > 0 ? video : true, audio: false });
      this.video.srcObject = this.stream;
    } else {
      if (!this.config.url) throw new Error('Video file input has no URL');
      this.video.src = this.config.url;
    }

    await new Promise<void>((resolve, reject) => {
      const onLoaded = () => {
        cleanup();
        resolve();
      };
      const onError = () => {
        cleanup();
        reject(new Error('Failed to load video source'));
      };
      const cleanup = () => {
        this.video.removeEventListener('loadedmetadata', onLoaded);
        this.video.removeEventListener('error', onError);
      };
      this.video.addEventListener('loadedmetadata', onLoaded);
      this.video.addEventListener('error', onError);
    });

    await this.video.play();
    this.texture = new THREE.VideoTexture(this.video);
    this.texture.minFilter = THREE.LinearFilter;
    this.texture.magFilter = THREE.LinearFilter;
    this.texture.generateMipmaps = false;
    this.ready = true;
  }

  getTexture(): THREE.VideoTexture | null {
    return this.ready ? this.texture : null;
  }

  getResolution(): { width: number; height: number } {
    return { width: this.video.videoWidth, height: this.video.videoHeight };
  }

  getDuration(): number {
    return this.config.type === 'camera' ? Infinity : this.video.duration;
  }

  getCurrentTime(): number {
    return this.video.currentTime;
  }

  play(): void {
    void this.video.play();
  }

  pause(): void {
    this.video.pause();
  }

  seek(t: number): void {
    this.video.currentTime = t;
  }

  setPlaybackRate(rate: number): void {
    this.video.playbackRate = rate;
  }

  setLoop(loop: boolean): void {
    this.video.loop = loop;
  }

  dispose(): void {
    this.video.pause();
    this.texture?.dispose();
    this.texture = null;
    if (this.stream) {
      for (const track of this.stream.getTracks()) track.stop();
      this.stream = null;
    }
    this.video.srcObject = null;
    this.video.removeAttribute('src');
    this.ready = false;
  }
}
