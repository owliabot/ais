use std::io::{self, BufRead, Write};

use ais_agent_host::{control::HostCommandService, events::HostRunEventService};

use crate::jsonl::{
    decode_inbound_line, encode_outbound_frame, JsonlInboundFrame, JsonlOutboundFrame,
    JsonlResponseFrame, JsonlServerErrorFrame,
};

#[derive(Debug, Default)]
pub struct JsonlServer;

impl JsonlServer {
    pub async fn serve<R, W, S>(&self, reader: R, mut writer: W, service: &mut S) -> io::Result<()>
    where
        R: BufRead,
        W: Write,
        S: HostCommandService + HostRunEventService,
    {
        for line in reader.lines() {
            let line = line?;
            if line.trim().is_empty() {
                continue;
            }

            match decode_inbound_line(&line) {
                Ok(JsonlInboundFrame::Command { command }) => {
                    let request_id = command.host_request_id.clone();
                    let outcome = service.handle(command).await;

                    self.write_frame(
                        &mut writer,
                        &JsonlOutboundFrame::Response(JsonlResponseFrame {
                            request_id,
                            response: outcome.response,
                        }),
                    )?;

                    for event in outcome.events {
                        self.write_frame(&mut writer, &JsonlOutboundFrame::Event { event })?;
                    }
                }
                Ok(JsonlInboundFrame::PollEvents { query }) => {
                    match service.list_events(query).await {
                        Ok(batch) => {
                            self.write_frame(
                                &mut writer,
                                &JsonlOutboundFrame::EventBatch { batch },
                            )?;
                        }
                        Err(error) => {
                            self.write_frame(
                                &mut writer,
                                &JsonlOutboundFrame::ServerError(JsonlServerErrorFrame {
                                    code: error.code,
                                    message: error.message,
                                }),
                            )?;
                        }
                    }
                }
                Err(error) => {
                    self.write_frame(
                        &mut writer,
                        &JsonlOutboundFrame::ServerError(JsonlServerErrorFrame {
                            code: "jsonl_decode_error".to_owned(),
                            message: error.to_string(),
                        }),
                    )?;
                }
            }
        }

        Ok(())
    }

    fn write_frame<W>(&self, writer: &mut W, frame: &JsonlOutboundFrame) -> io::Result<()>
    where
        W: Write,
    {
        let encoded = encode_outbound_frame(frame)
            .map_err(|error| io::Error::new(io::ErrorKind::InvalidData, error.to_string()))?;
        writer.write_all(encoded.as_bytes())?;
        writer.write_all(b"\n")?;
        writer.flush()?;
        Ok(())
    }
}
