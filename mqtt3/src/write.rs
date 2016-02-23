use byteorder::{WriteBytesExt, BigEndian};
use error::{Error, Result};
use std::io::{BufWriter, Write, Cursor};
use std::net::TcpStream;
use super::{PacketType, Header, QoS, Protocol, PacketIdentifier, MAX_PAYLOAD_SIZE};

use mqtt::{
    Packet,
    Connect,
    Connack,
    Publish,
    Subscribe,
    Suback,
    Unsubscribe
};

pub trait MqttWrite: WriteBytesExt {
    fn write_packet(&mut self, packet: &Packet) -> Result<()> {
        match packet {
            &Packet::Connect(ref connect) => {
                try!(self.write_u8(0b00010000));
                let prot_name = connect.protocol.name();
                let mut len = 8 + prot_name.len() + connect.client_id.len();
                if let Some(ref last_will) = connect.last_will {
                    len += 4 + last_will.topic.len() + last_will.message.len();
                }
                if let Some(ref username) = connect.username {
                    len += 2 + username.len();
                }
                if let Some(ref password) = connect.password {
                    len += 2 + password.len();
                }
                try!(self.write_remaining_length(len));
                try!(self.write_mqtt_string(prot_name));
                try!(self.write_u8(connect.protocol.level()));
                let mut connect_flags = 0;
                if connect.clean_session {
                    connect_flags |= 0x02;
                }
                if let Some(ref last_will) = connect.last_will {
                    connect_flags |= 0x04;
                    connect_flags |= last_will.qos.to_u8() << 3;
                    if last_will.retain {
                        connect_flags |= 0x20;
                    }
                }
                if let Some(_) = connect.password {
                    connect_flags |= 0x40;
                }
                if let Some(_) = connect.username {
                    connect_flags |= 0x80;
                }
                try!(self.write_u8(connect_flags));
                try!(self.write_u16::<BigEndian>(connect.keep_alive));
                try!(self.write_mqtt_string(connect.client_id.as_ref()));
                if let Some(ref last_will) = connect.last_will {
                    try!(self.write_mqtt_string(last_will.topic.as_ref()));
                    try!(self.write_mqtt_string(last_will.message.as_ref()));
                }
                if let Some(ref username) = connect.username {
                    try!(self.write_mqtt_string(username));
                }
                if let Some(ref password) = connect.password {
                    try!(self.write_mqtt_string(password));
                }
                Ok(())
            },
			&Packet::Connack(ref connack) => {
                try!(self.write(&[0x20, 0x02, connack.session_present as u8, connack.code.to_u8()]));
                Ok(())
            },
			&Packet::Publish(ref publish) => {
                try!(self.write_u8(0b00110000 | publish.retain as u8 | (publish.qos.to_u8() << 1) | ((publish.dup as u8) << 3)));
                try!(self.write_remaining_length(publish.topic_name.len() + 4 + publish.payload.len()));
                try!(self.write_mqtt_string(publish.topic_name.as_str()));
                if publish.qos != QoS::AtMostOnce {
                    if let Some(pid) = publish.pid {
                        try!(self.write_u16::<BigEndian>(pid.0));
                    }
                }
                try!(self.write(&publish.payload.as_ref()));
                Ok(())
            },
			&Packet::Puback(ref pid) => {
                try!(self.write(&[0x40, 0x02]));
                try!(self.write_u16::<BigEndian>(pid.0));
                Ok(())
            },
			&Packet::Pubrec(_) => Err(Error::UnsupportedPacketType),
			&Packet::Pubrel(ref pid) => {
                try!(self.write(&[0x62, 0x02]));
                try!(self.write_u16::<BigEndian>(pid.0));
                Ok(())
            },
			&Packet::Pubcomp(_) => Err(Error::UnsupportedPacketType),
			&Packet::Subscribe(ref subscribe) => {
                try!(self.write(&[0x82]));
                let mut len = 2;
                let topics: &Vec<(String, QoS)> = subscribe.topics.as_ref();
                for &(ref topic, _) in topics {
                    len += topic.len() + 3
                }
                try!(self.write_remaining_length(len));
                try!(self.write_u16::<BigEndian>(subscribe.pid.0));
                for &(ref topic, ref qos) in topics {
                    try!(self.write_mqtt_string(topic.as_str()));
                    try!(self.write_u8(qos.to_u8()));
                }
                Ok(())
            },
			&Packet::Suback(ref suback) => {
                try!(self.write(&[0x90]));
                try!(self.write_remaining_length(suback.return_codes.len() + 2));
                try!(self.write_u16::<BigEndian>(suback.pid.0));
                let payload: Vec<u8> = suback.return_codes.iter().map({ |&(err, qos)| ((err as u8) << 7) & qos.to_u8() }).collect();
                try!(self.write(&payload));
                Ok(())
            },
			&Packet::Unsubscribe(_) => Err(Error::UnsupportedPacketType),
			&Packet::Unsuback(ref pid) => {
                try!(self.write(&[0xB0, 0x02]));
                try!(self.write_u16::<BigEndian>(pid.0));
                Ok(())
            },
			&Packet::Pingreq => {
                try!(self.write(&[0xc0, 0]));
                Ok(())
            },
			&Packet::Pingresp => {
                try!(self.write(&[0xd0, 0]));
                Ok(())
            },
			&Packet::Disconnect => {
                try!(self.write(&[0xe0, 0]));
                Ok(())
            }
        }
    }

    fn write_mqtt_string(&mut self, string: &str) -> Result<()> {
        try!(self.write_u16::<BigEndian>(string.len() as u16));
        try!(self.write(string.as_bytes()));
        Ok(())
    }

    fn write_remaining_length(&mut self, len: usize) -> Result<()> {
        if len > MAX_PAYLOAD_SIZE {
            return Err(Error::PayloadTooLong);
        }

        let mut done = false;
        let mut x = len;

        while !done {
            let mut byte = (x % 128) as u8;
            x = x / 128;
            if x > 0 {
                byte = byte | 128;
            }
            try!(self.write_u8(byte));
            done = x <= 0;
        }

        Ok(())
    }
}

impl MqttWrite for TcpStream {}
impl MqttWrite for Cursor<Vec<u8>> {}
impl<T: Write> MqttWrite for BufWriter<T> {}

#[cfg(test)]
mod test {
    use std::io::Cursor;
    use std::sync::Arc;
    use super::{MqttWrite};
    use super::super::{Protocol, LastWill, QoS, PacketIdentifier, ConnectReturnCode};
    use super::super::mqtt::{
        Packet,
        Connect,
        Connack,
        Publish,
        Subscribe
    };

    #[test]
    fn write_packet_connect_mqtt_protocol_test() {
        let connect = Packet::Connect(Arc::new(Connect {
            protocol: Protocol::MQTT(4),
            keep_alive: 10,
            client_id: "test".to_owned(),
            clean_session: true,
            last_will: Some(LastWill {
                topic: "/a".to_owned(),
                message: "offline".to_owned(),
                retain: false,
                qos: QoS::AtLeastOnce
            }),
            username: Some("rust".to_owned()),
            password: Some("mq".to_owned())
        }));

        let mut stream = Cursor::new(Vec::new());
        stream.write_packet(&connect);

        assert_eq!(stream.get_ref().clone(), vec![0x10, 39,
            0x00, 0x04, 'M' as u8, 'Q' as u8, 'T' as u8, 'T' as u8,
            0x04,
            0b11001110, // +username, +password, -will retain, will qos=1, +last_will, +clean_session
            0x00, 0x0a, // 10 sec
            0x00, 0x04, 't' as u8, 'e' as u8, 's' as u8, 't' as u8, // client_id
            0x00, 0x02, '/' as u8, 'a' as u8, // will topic = '/a'
            0x00, 0x07, 'o' as u8, 'f' as u8, 'f' as u8, 'l' as u8, 'i' as u8, 'n' as u8, 'e' as u8, // will msg = 'offline'
            0x00, 0x04, 'r' as u8, 'u' as u8, 's' as u8, 't' as u8, // username = 'rust'
            0x00, 0x02, 'm' as u8, 'q' as u8 // password = 'mq'
        ]);
    }

    #[test]
    fn write_packet_connect_mqisdp_protocol_test() {
        let connect = Packet::Connect(Arc::new(Connect {
            protocol: Protocol::MQIsdp(3),
            keep_alive: 60,
            client_id: "test".to_owned(),
            clean_session: false,
            last_will: None,
            username: None,
            password: None
        }));

        let mut stream = Cursor::new(Vec::new());
        stream.write_packet(&connect);

        assert_eq!(stream.get_ref().clone(), vec![0x10, 18,
            0x00, 0x06, 'M' as u8, 'Q' as u8, 'I' as u8, 's' as u8, 'd' as u8, 'p' as u8,
            0x03,
            0b00000000, // -username, -password, -will retain, will qos=0, -last_will, -clean_session
            0x00, 0x3c, // 60 sec
            0x00, 0x04, 't' as u8, 'e' as u8, 's' as u8, 't' as u8 // client_id
        ]);
    }

    #[test]
    fn write_packet_connack_test() {
        let connack = Packet::Connack(Connack {
            session_present: true,
            code: ConnectReturnCode::Accepted
        });

        let mut stream = Cursor::new(Vec::new());
        stream.write_packet(&connack);

        assert_eq!(stream.get_ref().clone(), vec![0b00100000, 0x02, 0x01, 0x00]);
    }

    #[test]
    fn write_packet_publish_test() {
        let publish = Packet::Publish(Arc::new(Publish {
            dup: false,
            qos: QoS::AtLeastOnce,
            retain: false,
            topic_name: "a/b".to_owned(),
            pid: Some(PacketIdentifier(10)),
            payload: Arc::new(vec![0xF1, 0xF2, 0xF3, 0xF4])
        }));

        let mut stream = Cursor::new(Vec::new());
        stream.write_packet(&publish);

        assert_eq!(stream.get_ref().clone(), vec![0b00110010, 11, 0x00, 0x03, 'a' as u8, '/' as u8, 'b' as u8, 0x00, 0x0a, 0xF1, 0xF2, 0xF3, 0xF4]);
    }

    #[test]
    fn write_packet_subscribe_test() {
        let subscribe = Packet::Subscribe(Arc::new(Subscribe {
            pid: PacketIdentifier(260),
            topics: vec![
                ("a/+".to_owned(), QoS::AtMostOnce),
                ("#".to_owned(), QoS::AtLeastOnce),
                ("a/b/c".to_owned(), QoS::ExactlyOnce)
            ]
        }));

        let mut stream = Cursor::new(Vec::new());
        stream.write_packet(&subscribe);

        assert_eq!(stream.get_ref().clone(),vec![0b10000010, 20,
            0x01, 0x04, // pid = 260
            0x00, 0x03, 'a' as u8, '/' as u8, '+' as u8, // topic filter = 'a/+'
            0x00, // qos = 0
            0x00, 0x01, '#' as u8, // topic filter = '#'
            0x01, // qos = 1
            0x00, 0x05, 'a' as u8, '/' as u8, 'b' as u8, '/' as u8, 'c' as u8, // topic filter = 'a/b/c'
            0x02 // qos = 2
        ]);
    }
}
