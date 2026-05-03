use std::fmt;

#[derive(Debug, Clone)]
pub enum AmcpCommand {
    CgAdd {
        channel: u16,
        layer: u16,
        cg_layer: u16,
        template: String,
        play_on_load: bool,
        data: Option<String>,
    },
    CgUpdate {
        channel: u16,
        layer: u16,
        cg_layer: u16,
        data: String,
    },
    CgPlay {
        channel: u16,
        layer: u16,
        cg_layer: u16,
    },
    CgStop {
        channel: u16,
        layer: u16,
        cg_layer: u16,
    },
    CgClear {
        channel: u16,
        layer: u16,
    },
    Play {
        channel: u16,
        layer: u16,
        clip: Option<String>,
    },
    Load {
        channel: u16,
        layer: u16,
        clip: String,
    },
    Stop {
        channel: u16,
        layer: u16,
    },
    Clear {
        channel: u16,
        layer: Option<u16>,
    },
    Raw(String),
}

impl AmcpCommand {
    pub fn cg_add(channel: u16, layer: u16, template: &str) -> Self {
        Self::CgAdd {
            channel,
            layer,
            cg_layer: 0,
            template: template.to_owned(),
            play_on_load: true,
            data: None,
        }
    }

    pub fn cg_add_with_data(channel: u16, layer: u16, template: &str, data: &str) -> Self {
        Self::CgAdd {
            channel,
            layer,
            cg_layer: 0,
            template: template.to_owned(),
            play_on_load: true,
            data: Some(data.to_owned()),
        }
    }

    pub fn cg_update(channel: u16, layer: u16, data: &str) -> Self {
        Self::CgUpdate { channel, layer, cg_layer: 0, data: data.to_owned() }
    }

    pub fn cg_stop(channel: u16, layer: u16) -> Self {
        Self::CgStop { channel, layer, cg_layer: 0 }
    }
}

fn escape_data(data: &str) -> String {
    data.replace('\\', "\\\\").replace('"', "\\\"")
}

impl fmt::Display for AmcpCommand {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::CgAdd { channel, layer, cg_layer, template, play_on_load, data } => {
                let pol = if *play_on_load { 1 } else { 0 };
                write!(f, "CG {channel}-{layer} ADD {cg_layer} \"{template}\" {pol}")?;
                if let Some(d) = data {
                    write!(f, " \"{}\"", escape_data(d))?;
                }
                Ok(())
            }
            Self::CgUpdate { channel, layer, cg_layer, data } => {
                write!(f, "CG {channel}-{layer} UPDATE {cg_layer} \"{}\"", escape_data(data))
            }
            Self::CgPlay { channel, layer, cg_layer } => {
                write!(f, "CG {channel}-{layer} PLAY {cg_layer}")
            }
            Self::CgStop { channel, layer, cg_layer } => {
                write!(f, "CG {channel}-{layer} STOP {cg_layer}")
            }
            Self::CgClear { channel, layer } => write!(f, "CG {channel}-{layer} CLEAR"),
            Self::Play { channel, layer, clip } => {
                write!(f, "PLAY {channel}-{layer}")?;
                if let Some(c) = clip {
                    write!(f, " \"{c}\"")?;
                }
                Ok(())
            }
            Self::Load { channel, layer, clip } => write!(f, "LOAD {channel}-{layer} \"{clip}\""),
            Self::Stop { channel, layer } => write!(f, "STOP {channel}-{layer}"),
            Self::Clear { channel, layer } => {
                write!(f, "CLEAR {channel}")?;
                if let Some(l) = layer {
                    write!(f, "-{l}")?;
                }
                Ok(())
            }
            Self::Raw(cmd) => write!(f, "{cmd}"),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn cg_add_basic() {
        let cmd = AmcpCommand::cg_add(1, 10, "lower_third");
        assert_eq!(cmd.to_string(), r#"CG 1-10 ADD 0 "lower_third" 1"#);
    }

    #[test]
    fn cg_add_with_data() {
        let cmd = AmcpCommand::cg_add_with_data(1, 10, "lower_third", r#"{"name":"Wilson"}"#);
        assert_eq!(cmd.to_string(), r#"CG 1-10 ADD 0 "lower_third" 1 "{\"name\":\"Wilson\"}""#);
    }

    #[test]
    fn cg_update() {
        let cmd = AmcpCommand::cg_update(1, 10, r#"{"name":"Chris"}"#);
        assert_eq!(cmd.to_string(), r#"CG 1-10 UPDATE 0 "{\"name\":\"Chris\"}""#);
    }

    #[test]
    fn cg_stop() {
        let cmd = AmcpCommand::cg_stop(1, 10);
        assert_eq!(cmd.to_string(), "CG 1-10 STOP 0");
    }

    #[test]
    fn play_with_clip() {
        let cmd = AmcpCommand::Play { channel: 1, layer: 1, clip: Some("AMB".to_owned()) };
        assert_eq!(cmd.to_string(), r#"PLAY 1-1 "AMB""#);
    }

    #[test]
    fn clear_channel() {
        let cmd = AmcpCommand::Clear { channel: 1, layer: None };
        assert_eq!(cmd.to_string(), "CLEAR 1");
    }
}
