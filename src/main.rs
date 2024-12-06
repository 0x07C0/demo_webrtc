use std::collections::HashMap;
use std::sync::Arc;

use anyhow::Result;
use demo_webrtc::http_server;
use tokio::time::Duration;
use webrtc::api::interceptor_registry::register_default_interceptors;
use webrtc::api::media_engine::{MediaEngine, MIME_TYPE_OPUS, MIME_TYPE_VP8};
use webrtc::api::APIBuilder;
use webrtc::ice_transport::ice_server::RTCIceServer;
use webrtc::interceptor::registry::Registry;
use webrtc::peer_connection::configuration::RTCConfiguration;
use webrtc::peer_connection::peer_connection_state::RTCPeerConnectionState;
use webrtc::rtcp::payload_feedbacks::picture_loss_indication::PictureLossIndication;
use webrtc::rtp_transceiver::rtp_codec::{
  RTCRtpCodecCapability, RTCRtpCodecParameters, RTPCodecType,
};
use webrtc::track::track_local::track_local_static_rtp::TrackLocalStaticRTP;
use webrtc::track::track_local::{TrackLocal, TrackLocalWriter};

#[tokio::main]
async fn main() -> Result<()> {
  let mut media = MediaEngine::default();

  media.register_codec(
    RTCRtpCodecParameters {
      capability: RTCRtpCodecCapability {
        mime_type: MIME_TYPE_OPUS.to_owned(),
        ..Default::default()
      },
      payload_type: 120,
      ..Default::default()
    },
    RTPCodecType::Audio,
  )?;

  media.register_codec(
    RTCRtpCodecParameters {
      capability: RTCRtpCodecCapability {
        mime_type: MIME_TYPE_VP8.to_owned(),
        clock_rate: 90000,
        channels: 0,
        sdp_fmtp_line: "".to_owned(),
        rtcp_feedback: vec![],
      },
      payload_type: 96,
      ..Default::default()
    },
    RTPCodecType::Video,
  )?;

  let mut registry = Registry::new();
  registry = register_default_interceptors(registry, &mut media)?;
  let api = APIBuilder::new()
    .with_media_engine(media)
    .with_interceptor_registry(registry)
    .build();

  let config = RTCConfiguration {
    ice_servers: vec![RTCIceServer {
      urls: vec!["stun:stun.l.google.com:19302".to_owned()],
      ..Default::default()
    }],
    ..Default::default()
  };

  let peer_connection = Arc::new(api.new_peer_connection(config).await?);
  let mut output_tracks = HashMap::new();
  for s in ["audio", "video"] {
    let output_track = Arc::new(TrackLocalStaticRTP::new(
      RTCRtpCodecCapability {
        mime_type: if s == "video" {
          MIME_TYPE_VP8.to_owned()
        } else {
          MIME_TYPE_OPUS.to_owned()
        },
        ..Default::default()
      },
      format!("track-{s}"),
      "webrtc-rs".to_owned(),
    ));

    let rtp_sender = peer_connection
      .add_track(Arc::clone(&output_track) as Arc<dyn TrackLocal + Send + Sync>)
      .await?;

    let m = s.to_owned();
    tokio::spawn(async move {
      let mut rtcp_buf = vec![0u8; 1500];
      while let Ok((_, _)) = rtp_sender.read(&mut rtcp_buf).await {}
      println!("{m} rtp_sender.read loop exit");
      Result::<()>::Ok(())
    });

    output_tracks.insert(s.to_owned(), output_track);
  }

  let pc = Arc::downgrade(&peer_connection);
  peer_connection.on_track(Box::new(move |track, _, _| {
    let media_ssrc = track.ssrc();

    if track.kind() == RTPCodecType::Video {
      let pc2 = pc.clone();
      tokio::spawn(async move {
        let mut result = Result::<usize>::Ok(0);
        while result.is_ok() {
          let timeout = tokio::time::sleep(Duration::from_secs(3));
          tokio::pin!(timeout);

          tokio::select! {
              _ = timeout.as_mut() =>{
                  if let Some(pc) = pc2.upgrade(){
                      result = pc.write_rtcp(&[Box::new(PictureLossIndication{
                              sender_ssrc: 0,
                              media_ssrc,
                      })]).await.map_err(Into::into);
                  }else{
                      break;
                  }
              }
          };
        }
      });
    }

    let kind = if track.kind() == RTPCodecType::Audio {
      "audio"
    } else {
      "video"
    };
    let output_track = if let Some(output_track) = output_tracks.get(kind) {
      Arc::clone(output_track)
    } else {
      println!("output_track not found for type = {kind}");
      return Box::pin(async {});
    };

    let output_track2 = Arc::clone(&output_track);
    tokio::spawn(async move {
      println!(
        "Track has started, of type {}: {}",
        track.payload_type(),
        track.codec().capability.mime_type
      );
      while let Ok((rtp, _)) = track.read_rtp().await {
        if let Err(err) = output_track2.write_rtp(&rtp).await {
          println!("output track write_rtp got error: {err}");
          break;
        }
      }

      println!(
        "on_track finished, of type {}: {}",
        track.payload_type(),
        track.codec().capability.mime_type
      );
    });

    Box::pin(async {})
  }));

  let (done_tx, mut done_rx) = tokio::sync::mpsc::channel::<()>(1);

  peer_connection.on_peer_connection_state_change(Box::new(move |s: RTCPeerConnectionState| {
    println!("Peer Connection State has changed: {s}");

    if s == RTCPeerConnectionState::Failed {
      println!("Peer Connection has gone to failed exiting");
      let _ = done_tx.try_send(());
    }

    Box::pin(async {})
  }));

  let (mut rx, tx) = http_server(8080).await;

  println!("Demo live video started at http://localhost:8080/");

  println!("Waiting for an offer");
  let Some(offer) = rx.recv().await else {
    panic!("");
  };

  peer_connection.set_remote_description(offer).await?;
  let answer = peer_connection.create_answer(None).await?;
  let mut gather_complete = peer_connection.gathering_complete_promise().await;
  peer_connection.set_local_description(answer).await?;
  let _ = gather_complete.recv().await;

  if let Some(local_desc) = peer_connection.local_description().await {
    let json_str = serde_json::to_string(&local_desc)?;
    let b64 = demo_webrtc::encode(&json_str);
    tx.send(b64).await?;
  } else {
    println!("Generate local_description failed!");
  }

  println!("Press ctrl-c to stop");

  tokio::select! {
    _ = done_rx.recv() => {
      println!("Received done signal!");
    }
    _ = tokio::signal::ctrl_c() => {
      println!("Exiting");
    }
  };

  peer_connection.close().await?;

  Ok(())
}
