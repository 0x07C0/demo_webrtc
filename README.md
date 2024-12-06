# WebRTC video/audio stream

A simple WebRTC streaming server. It streams video and audio that it receives from a browser client.

## Try it out!
1. Install [Rust](https://rustup.rs/)
2. Run the app:
```bash
$ cargo run --release
```
3. Open the `http://localhost:8080` in your browser
4. Allow camera and microphone access
5. After the local session description is generated you'll see the video 
stream created from your camera streamed over WebRTC
