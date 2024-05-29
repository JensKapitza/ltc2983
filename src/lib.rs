//! LTC2983
//!
//! This create provides a complete implemantition of the communication with the
//! `LTC2983` (Multi Sensor High Accuracy Digital Temperature Measurement System) via
//! SPI. Not all sensor types are supported yet.
//!
//! Contributions welcome 💪
//!
//! - [x] Theromcouple J,K,E,N,R,S,T,B
//! - [ ] Custom Thermocouple
//! - [x] RTD
//! - [ ] Thermistor
//! - [x] Sense Resistor
//! - [x] Diode
//! - [ ] Direct ADC
//!
//!# Example
//!``` rust
//!    let mut ltc = LTC2983::new(device);
//!
//!    let _ = ltc.setup_channel(ltc2983::ThermalProbeType::Diode(ltc2983::DiodeParameters::default().ideality_factor(1.).excitation_current(ltc2983::DiodeExcitationCurrent::I20uA).num_reading(ltc2983::DiodeReadingCount::READ3)), ltc2983::LTC2983Channel::CH2);
//!    let _ = ltc.setup_channel(ltc2983::ThermalProbeType::Thermocouple_T(ThermocoupleParameters::default().cold_junction(ltc2983::LTC2983Channel::CH2)), ltc2983::LTC2983Channel::CH1);
//!
//!    loop {
//!        let _ = ltc.start_conversion(ltc2983::LTC2983Channel::CH1);
//!        let mut status = ltc.status().unwrap();
//!        while !status.done() {
//!            status = ltc.status().unwrap();
//!        }
//!        let result = ltc.read_temperature(ltc2983::LTC2983Channel::CH1);
//!        println!("{result:#?}");
//!        sleep(Duration::new(1, 0));
//!    }
//!
//!```

use std::{convert::TryInto,thread};

use std::time::{Duration};
use bytebuffer::ByteBuffer;
use embedded_hal::spi::{SpiDevice, SpiBus};
use fixed::{FixedU32, types::extra::{U10, U20}, FixedI32};
use serde::{Serialize, Deserialize};
use thiserror::Error;

const LTC2983_WRITE: u8 = 0x2;
const LTC2983_READ: u8 = 0x3;

const STATUS_REGISTER: u16 = 0x000;
//const GLOBAL_CONFIG_REGISTER: u16 = 0x0F0;
const MULTI_CHANNEL_MASK_REGISTER: u16 = 0x0F4;

#[derive(Debug)]
pub enum SensorConfiguration {
    SingleEnded,
    Differential
}

impl Default for SensorConfiguration {
    fn default() -> Self {
        Self::SingleEnded
    }
}

impl SensorConfiguration {
    pub fn identifier(&self) -> u64 {
        match self {
            SensorConfiguration::SingleEnded => 1,
            SensorConfiguration::Differential => 0,
        }
    }
}

#[derive(Debug)]
pub struct ThermocoupleParameters {
    cold_junction_channel: Option<LTC2983Channel>,
    sensor_configuration: SensorConfiguration,
    oc_current: LTC2983OcCurrent,
    custom_address: Option<u16>
}

impl Default for ThermocoupleParameters {
    fn default() -> Self {
        Self { cold_junction_channel: None,
               sensor_configuration: Default::default(),
               oc_current: Default::default(),
               custom_address: None }
    }
}

impl ThermocoupleParameters {
    pub fn cold_junction(mut self, chan: LTC2983Channel) -> Self {
        self.cold_junction_channel = Some(chan);
        self
    }

    pub fn sensor_configuration(mut self, config: SensorConfiguration) -> Self {
        self.sensor_configuration = config;
        self
    }

    pub fn custom_address(mut self, addr: u16) -> Self {
        self.custom_address = Some(addr);
        self
    }

    pub fn oc_current(mut self, oc_current: LTC2983OcCurrent) -> Self {
        self.oc_current = oc_current;
        self
    }

    pub fn config_to_bits(&self) -> u64 {
        0x0 | (self.sensor_configuration.identifier() << 3) | self.oc_current.identifier()
    }
}

#[derive(Debug)]
#[allow(non_camel_case_types)]
pub enum RTDCurve {
    EuropeanStandard,
    American,
    Japanese,
    ITS_90
}

impl Default for RTDCurve {
    fn default() -> Self {
        Self::EuropeanStandard
    }
}

impl RTDCurve {
    pub fn identifier(&self) -> u64 {
        match self {
            RTDCurve::EuropeanStandard  => 0,
            RTDCurve::American          => 1,
            RTDCurve::Japanese          => 2,
            RTDCurve::ITS_90            => 3,
        }
    }
}

#[derive(Debug, PartialEq)]
pub enum RTDWireCount {
    Wire2,
    Wire3,
    Wire4,
    Wire4KelvinRsense
}

impl RTDWireCount {
    pub fn identifier(&self) -> u64 {
        match self {
            RTDWireCount::Wire2             => 0,
            RTDWireCount::Wire3             => 1,
            RTDWireCount::Wire4             => 2,
            RTDWireCount::Wire4KelvinRsense => 3,
        }
    }
}

impl Default for RTDWireCount {
    fn default() -> Self {
        Self::Wire2
    }
}

#[derive(Debug)]
pub struct RTDSensorConfiguration {
    wire_cnt: RTDWireCount,
    external: bool,
    current_source_rotation: bool
}

impl Default for RTDSensorConfiguration {
    fn default() -> Self {
        Self {
            wire_cnt: Default::default(),
            external: false,
            current_source_rotation: false
        }
    }
}

impl RTDSensorConfiguration {
    pub fn wire_cnt(mut self, wire_cnt: RTDWireCount) -> Self { self.wire_cnt = wire_cnt; self }
    pub fn external(mut self, external: bool) -> Self { self.external = external; self }
    pub fn current_source_rotation(mut self, current_src_rotation: bool) -> Self { self.current_source_rotation = current_src_rotation; self }

    pub fn to_bits(&self) -> u64 {
        let mut bits = 0x0;
        bits = (bits | self.wire_cnt.identifier()) << 2;
        if self.current_source_rotation && self.wire_cnt != RTDWireCount::Wire2 && self.wire_cnt != RTDWireCount::Wire3 { // current source rotation is not support in 2 or 3 wire RTDs
            bits = (bits | 0x1) << 1;
        } else {
            if !self.external {
                bits = bits | 0x1
            }
        }

        bits
    }
}

#[derive(Debug)]
pub enum RTDExcitationCurrent {
    I5uA,
    I10uA,
    I25uA,
    I50uA,
    I100uA,
    I250uA,
    I500uA,
    I1mA
}

impl Default for RTDExcitationCurrent {
    fn default() -> Self {
        Self::I5uA
    }
}

impl RTDExcitationCurrent {
    pub fn identifier(&self) -> u64 {
       match self {
        RTDExcitationCurrent::I5uA   => 1,
        RTDExcitationCurrent::I10uA  => 2,
        RTDExcitationCurrent::I25uA  => 3,
        RTDExcitationCurrent::I50uA  => 4,
        RTDExcitationCurrent::I100uA => 5,
        RTDExcitationCurrent::I250uA => 6,
        RTDExcitationCurrent::I500uA => 7,
        RTDExcitationCurrent::I1mA   => 8,
    }
    }
}

#[derive(Debug)]
pub struct RTDParameters {
    r_sense_channel: LTC2983Channel,
    sensor_configuration: RTDSensorConfiguration,
    excitation_current: RTDExcitationCurrent,
    curve: RTDCurve,
    custom_address: Option<u16>
}

impl Default for RTDParameters {
    fn default() -> Self {
        Self {
            r_sense_channel: LTC2983Channel::CH2,
            sensor_configuration: Default::default(),
            excitation_current: Default::default(),
            curve: Default::default(),
            custom_address: None
        }
    }
}

impl RTDParameters {
    pub fn curve(mut self, curve: RTDCurve) -> Self { self.curve = curve; self}
    pub fn excitation_current(mut self, excitation_current: RTDExcitationCurrent) -> Self { self.excitation_current = excitation_current; self }
    pub fn sensor_configuration(mut self, config: RTDSensorConfiguration) -> Self { self.sensor_configuration = config; self }
    pub fn channel(mut self, channel: LTC2983Channel) -> Self {
        if channel == LTC2983Channel::CH1 {
            panic!("CH1 can not be used, because there is no channel 0 and the value here indicates that the resistor is between channel x and x-1!!!!")
        } else {
            self.r_sense_channel = channel;
            self
        }
    }
}

#[derive(Debug)]
pub enum DiodeReadingCount {
    READ2,
    READ3
}

impl Default for DiodeReadingCount {
    fn default() -> Self {
        Self::READ2
    }
}

impl DiodeReadingCount {
    pub fn identifier(&self) -> u64 {
        match self {
            DiodeReadingCount::READ2 => 0,
            DiodeReadingCount::READ3 => 1,
        }
    }
}

#[derive(Debug)]
pub enum DiodeExcitationCurrent {
    I10uA,
    I20uA,
    I40uA,
    I80uA
}

impl Default for DiodeExcitationCurrent {
    fn default() -> Self {
        Self::I10uA
    }
}

impl DiodeExcitationCurrent {
    pub fn identifier(&self) -> u64 {
        match self {
            DiodeExcitationCurrent::I10uA => 0,
            DiodeExcitationCurrent::I20uA => 1,
            DiodeExcitationCurrent::I40uA => 2,
            DiodeExcitationCurrent::I80uA => 3,
        }
    }
}

#[derive(Debug)]
pub struct DiodeParameters {
    sensor_configuration: SensorConfiguration,
    num_reading: DiodeReadingCount,
    avg: bool,
    excitation_current: DiodeExcitationCurrent,
    idealitiy_factor: Option<f32>
}

impl Default for DiodeParameters {
    fn default() -> Self {
        Self {
            sensor_configuration: Default::default(),
            num_reading: Default::default(),
            excitation_current: Default::default(),
            idealitiy_factor: None,
            avg: true,
        }
    }
}

impl DiodeParameters {
    pub fn sensor_configuration(mut self, config: SensorConfiguration) -> Self {
        self.sensor_configuration = config;
        self
    }

    pub fn num_reading(mut self, cnt: DiodeReadingCount) -> Self {
        self.num_reading = cnt;
        self
    }

    pub fn excitation_current(mut self, current: DiodeExcitationCurrent) -> Self {
        self.excitation_current = current;
        self
    }

    pub fn use_avg(mut self, flag: bool) -> Self {
        self.avg = flag;
        self
    }

    pub fn ideality_factor(mut self, factor: f32) -> Self {
        self.idealitiy_factor = Some(factor);
        self
    }

    pub fn to_bits(&self) -> u64 {
        0x0 | (self.sensor_configuration.identifier() << 26)
            | (self.num_reading.identifier() << 25)
            | ((self.avg as u64) << 24)
            | (self.excitation_current.identifier() << 22)
            | ( match self.idealitiy_factor {
                None => 0x0,
                Some(factor) => {
                    let factor_fixed_point = FixedU32::<U20>::from_num(factor);
                    (factor_fixed_point.to_bits() & 0x3fffff) as u64 //mask the upper bits to only include the lower 22 bits
                }
            })
    }
}


#[allow(non_camel_case_types)]
#[derive(Debug)]
pub enum ThermalProbeType {
    Thermocouple_J(ThermocoupleParameters),
    Thermocouple_K(ThermocoupleParameters),
    Thermocouple_E(ThermocoupleParameters),
    Thermocouple_N(ThermocoupleParameters),
    Thermocouple_R(ThermocoupleParameters),
    Thermocouple_S(ThermocoupleParameters),
    Thermocouple_T(ThermocoupleParameters),
    Thermocouple_B(ThermocoupleParameters),
    RTD_PT10(RTDParameters),
    RTD_PT50(RTDParameters),
    RTD_PT100(RTDParameters),
    RTD_PT200(RTDParameters),
    RTD_PT500(RTDParameters),
    RTD_PT1000(RTDParameters),
    RTD_1000(RTDParameters),
    RTD_NI120(RTDParameters),
    Thermistor_44004_44033,
    Thermistor_44005_44030,
    Thermistor_44007_44034,
    Thermistor_44006_44031,
    Thermistor_44008_44032,
    Thermistor_YSI400,
    Thermistor_Spectrum,
    Diode(DiodeParameters),
    SenseResistor(f32)
}

impl ThermalProbeType {
    pub fn identifier(&self) -> u64 {
        match self {
            ThermalProbeType::Thermocouple_J(_)      => 1,
            ThermalProbeType::Thermocouple_K(_)      => 2,
            ThermalProbeType::Thermocouple_E(_)      => 3,
            ThermalProbeType::Thermocouple_N(_)      => 4,
            ThermalProbeType::Thermocouple_R(_)      => 5,
            ThermalProbeType::Thermocouple_S(_)      => 6,
            ThermalProbeType::Thermocouple_T(_)      => 7,
            ThermalProbeType::Thermocouple_B(_)      => 8,
            ThermalProbeType::RTD_PT10(_)            => 10,
            ThermalProbeType::RTD_PT50(_)            => 11,
            ThermalProbeType::RTD_PT100(_)           => 12,
            ThermalProbeType::RTD_PT200(_)           => 13,
            ThermalProbeType::RTD_PT500(_)           => 14,
            ThermalProbeType::RTD_PT1000(_)          => 15,
            ThermalProbeType::RTD_1000(_)            => 16,
            ThermalProbeType::RTD_NI120(_)           => 17,
            ThermalProbeType::Thermistor_44004_44033 => 19,
            ThermalProbeType::Thermistor_44005_44030 => 20,
            ThermalProbeType::Thermistor_44007_44034 => 21,
            ThermalProbeType::Thermistor_44006_44031 => 22,
            ThermalProbeType::Thermistor_44008_44032 => 23,
            ThermalProbeType::Thermistor_YSI400      => 24,
            ThermalProbeType::Thermistor_Spectrum    => 25,
            ThermalProbeType::Diode(_)               => 28,
            ThermalProbeType::SenseResistor(_)       => 29
        }
    }
}

#[derive(Debug, Serialize, Deserialize, Clone)]
pub enum LTC2983Result {
    Invalid(u8),
    Suspect(f32, u8),
    Valid(f32)
}

impl From<[u8; 4]> for LTC2983Result {
    fn from(bytes: [u8; 4]) -> Self {
        let value = FixedI32::<U10>::from_be_bytes(reformat_fixedf24_to_fixed_f32(bytes[1..=3].try_into().unwrap()));
        let error_code = bytes[0];
        if error_code == 0x01 { // indicates valid result
            LTC2983Result::Valid(value.to_num())
        } else if error_code & 0xe != 0 { //if any of the upper three bits of the error code are set then the result is invalid
            LTC2983Result::Invalid(error_code)
        } else { // in all other cases the reading should regarded as suspect
            LTC2983Result::Suspect(value.to_num(), error_code)
        }
    }
}

#[derive(Debug, Copy, Clone, PartialEq)]
pub enum LTC2983Channel {
    CH1,
    CH2,
    CH3,
    CH4,
    CH5,
    CH6,
    CH7,
    CH8,
    CH9,
    CH10,
    CH11,
    CH12,
    CH13,
    CH14,
    CH15,
    CH16,
    CH17,
    CH18,
    CH19,
    CH20
}

impl LTC2983Channel {
    pub fn start_address(&self) -> u16 {
        match self {
            LTC2983Channel::CH1  => 0x200,
            LTC2983Channel::CH2  => 0x204,
            LTC2983Channel::CH3  => 0x208,
            LTC2983Channel::CH4  => 0x20C,
            LTC2983Channel::CH5  => 0x210,
            LTC2983Channel::CH6  => 0x214,
            LTC2983Channel::CH7  => 0x218,
            LTC2983Channel::CH8  => 0x21C,
            LTC2983Channel::CH9  => 0x220,
            LTC2983Channel::CH10 => 0x224,
            LTC2983Channel::CH11 => 0x228,
            LTC2983Channel::CH12 => 0x22C,
            LTC2983Channel::CH13 => 0x230,
            LTC2983Channel::CH14 => 0x234,
            LTC2983Channel::CH15 => 0x238,
            LTC2983Channel::CH16 => 0x23C,
            LTC2983Channel::CH17 => 0x240,
            LTC2983Channel::CH18 => 0x244,
            LTC2983Channel::CH19 => 0x248,
            LTC2983Channel::CH20 => 0x24C
        }
    }

    pub fn result_address(&self) -> u16 {
        match self {
            LTC2983Channel::CH1  => 0x010,
            LTC2983Channel::CH2  => 0x014,
            LTC2983Channel::CH3  => 0x018,
            LTC2983Channel::CH4  => 0x01C,
            LTC2983Channel::CH5  => 0x020,
            LTC2983Channel::CH6  => 0x024,
            LTC2983Channel::CH7  => 0x028,
            LTC2983Channel::CH8  => 0x02C,
            LTC2983Channel::CH9  => 0x030,
            LTC2983Channel::CH10 => 0x034,
            LTC2983Channel::CH11 => 0x038,
            LTC2983Channel::CH12 => 0x03C,
            LTC2983Channel::CH13 => 0x040,
            LTC2983Channel::CH14 => 0x044,
            LTC2983Channel::CH15 => 0x048,
            LTC2983Channel::CH16 => 0x04C,
            LTC2983Channel::CH17 => 0x050,
            LTC2983Channel::CH18 => 0x054,
            LTC2983Channel::CH19 => 0x058,
            LTC2983Channel::CH20 => 0x05C,
        }
    }

    pub fn identifier(&self) -> u64 {
        match self {
            LTC2983Channel::CH1  => 1,
            LTC2983Channel::CH2  => 2,
            LTC2983Channel::CH3  => 3,
            LTC2983Channel::CH4  => 4,
            LTC2983Channel::CH5  => 5,
            LTC2983Channel::CH6  => 6,
            LTC2983Channel::CH7  => 7,
            LTC2983Channel::CH8  => 8,
            LTC2983Channel::CH9  => 9,
            LTC2983Channel::CH10 => 10,
            LTC2983Channel::CH11 => 11,
            LTC2983Channel::CH12 => 12,
            LTC2983Channel::CH13 => 13,
            LTC2983Channel::CH14 => 14,
            LTC2983Channel::CH15 => 15,
            LTC2983Channel::CH16 => 16,
            LTC2983Channel::CH17 => 17,
            LTC2983Channel::CH18 => 18,
            LTC2983Channel::CH19 => 19,
            LTC2983Channel::CH20 => 20,
        }
    }

    pub fn mask(&self) -> u32 {
       0x1 << (self.identifier() - 1)
    }
}

#[derive(Debug)]
#[allow(dead_code)]
pub struct LTC2983Status {
    start: bool,
    done: bool,
    //1 bit unused
    channel_selection: u8
}

impl LTC2983Status {
    pub fn done(&self) -> bool {
        self.done
    }
}

impl From<u8> for LTC2983Status {
    fn from(data: u8) -> Self {
        LTC2983Status {
            start: data & 0x80 == 0x80,
            done: data & 0x40 == 0x40,
            channel_selection: data & 0x1f
        }
    }
}

#[derive(Debug)]
pub enum LTC2983OcCurrent {
    External,
    I10uA,
    I100uA,
    I500uA,
    I1mA
}

impl Default for LTC2983OcCurrent {
    fn default() -> Self {
        Self::I10uA
    }
}

impl LTC2983OcCurrent {
    pub fn identifier(&self) -> u64 {
        match self {
            LTC2983OcCurrent::External => 0,
            LTC2983OcCurrent::I10uA => 4,
            LTC2983OcCurrent::I100uA => 5,
            LTC2983OcCurrent::I500uA => 6,
            LTC2983OcCurrent::I1mA => 7,
        }
    }
}

#[derive(Debug, Error)]
pub enum LTC2983Error<SPI> {
    #[error("SPI communication error: {0:?}")]
    SpiError(#[from] SPI),
    #[error("Channel {0:?} not configured!")]
    ChannelUnconfigured(LTC2983Channel),
    #[error("Error while calculating average from mutliple rounds of readouts.")]
    AvgCalculationError
}

pub struct LTC2983<SPI> {
    spi_device: SPI,
}

impl<SPI> LTC2983<SPI> where SPI: SpiDevice {
    pub fn new(spi_device: SPI) -> Self {
        LTC2983 { spi_device }
    }

    //read device satatus
    pub fn status(&mut self) -> Result<LTC2983Status, LTC2983Error<SPI::Error>> {
        let mut read_status_bytes = ByteBuffer::new();
        read_status_bytes.write_u8(LTC2983_READ);
        read_status_bytes.write_u16(STATUS_REGISTER);
        read_status_bytes.write_u8(0x0); //Dummy Data

        let mut recv: [u8; 4] = [0, 0, 0, 0];
        match self.spi_device.transfer(&mut recv, read_status_bytes.as_bytes()) {
            Ok(_) => {
                Ok(LTC2983Status::from(recv[3]))
            }
            Err(err) => Err(LTC2983Error::SpiError(err))
        }

    }

    //write channel configuration
    pub fn setup_channel(&mut self,
                         probe: ThermalProbeType,
                         channel: &LTC2983Channel) -> Result<(), LTC2983Error<SPI::Error>>
    {
        match &probe {
            ThermalProbeType::Thermocouple_J(param) |
            ThermalProbeType::Thermocouple_K(param) |
            ThermalProbeType::Thermocouple_E(param) |
            ThermalProbeType::Thermocouple_N(param) |
            ThermalProbeType::Thermocouple_R(param) |
            ThermalProbeType::Thermocouple_S(param) |
            ThermalProbeType::Thermocouple_T(param) |
            ThermalProbeType::Thermocouple_B(param) => {
                let mut write_sequence = ByteBuffer::new();
                write_sequence.write_u8(LTC2983_WRITE);              //the first byte of the communication indicates a read or write operation
                write_sequence.write_u16(channel.start_address());   //the second two bytes hold the address to ẁrite to
                // The 32 bit data to be written to the channel configuration register has the following format for thermocouples
                // |31-27| Thermocouple Type
                write_sequence.write_bits(probe.identifier(), 5);
                // |26-22| Could Junction Channel ID -> if no cold junction compensation is used this value will be 0
                write_sequence.write_bits(match &param.cold_junction_channel { None => 0, Some(chan) => chan.identifier() }, 5);
                // |21-18| Sensor Configuration
                write_sequence.write_bits(param.config_to_bits(), 4);
                // |17-12| Unused => equals 0
                write_sequence.write_bits(0, 6);
                // |11-0| Custom Thermocouple Data Pointer
                write_sequence.write_bits(match &param.custom_address { None => 0, Some(addr) => *addr}.into(), 12);

                self.spi_device.write(write_sequence.as_bytes())?;
                Ok(())
            }
            ThermalProbeType::RTD_PT10(param)   |
            ThermalProbeType::RTD_PT50(param)   |
            ThermalProbeType::RTD_PT100(param)  |
            ThermalProbeType::RTD_PT200(param)  |
            ThermalProbeType::RTD_PT500(param)  |
            ThermalProbeType::RTD_PT1000(param) |
            ThermalProbeType::RTD_1000(param)   |
            ThermalProbeType::RTD_NI120(param)  => {
                let mut write_sequence = ByteBuffer::new();
                write_sequence.write_u8(LTC2983_WRITE);              //the first byte of the communication indicates a read or write operation
                write_sequence.write_u16(channel.start_address());   //the second two bytes hold the address to ẁrite to
                // The 32 bit data to be written to the channel configuration register has the following format for thermocouples
                // |31-27| RTD Type
                write_sequence.write_bits(probe.identifier(), 5);
                // |26-22| Rsense Channel Assignment
                write_sequence.write_bits(param.r_sense_channel.identifier(), 5);
                // |21-18| Sensor Configuration
                write_sequence.write_bits(param.sensor_configuration.to_bits(), 4);
                // |17-14| Excitation Current
                write_sequence.write_bits(param.excitation_current.identifier(), 4);
                // |13-12| Curve
                write_sequence.write_bits(param.curve.identifier(), 2);
                // |11-0| Custom RTD Data Pointer
                write_sequence.write_bits(match &param.custom_address { None => 0, Some(addr) => *addr}.into(), 12);

                self.spi_device.write(write_sequence.as_bytes())?;
                Ok(())
            }
            ThermalProbeType::Thermistor_44004_44033 |
            ThermalProbeType::Thermistor_44005_44030 |
            ThermalProbeType::Thermistor_44007_44034 |
            ThermalProbeType::Thermistor_44006_44031 |
            ThermalProbeType::Thermistor_44008_44032 |
            ThermalProbeType::Thermistor_YSI400      |
            ThermalProbeType::Thermistor_Spectrum    => {
                unimplemented!();
            }
            ThermalProbeType::Diode(param) => {
                let mut write_sequence = ByteBuffer::new();
                write_sequence.write_u8(LTC2983_WRITE);              //the first byte of the communication indicates a read or write operation
                write_sequence.write_u16(channel.start_address());   //the second two bytes hold the address to ẁrite to
                write_sequence.write_bits(probe.identifier(), 5);
                write_sequence.write_bits(param.to_bits(), 27);

                self.spi_device.write(write_sequence.as_bytes())?;
                Ok(())
            }
            ThermalProbeType::SenseResistor(resistance) => {
                let mut write_sequence = ByteBuffer::new();
                write_sequence.write_u8(LTC2983_WRITE);              //the first byte of the communication indicates a read or write operation
                write_sequence.write_u16(channel.start_address());   //the second two bytes hold the address to ẁrite to
                // The 32 bit data to be written to the channel configuration register has the following format for sense resistors
                // |31-27| Thermocouple Type
                write_sequence.write_bits(probe.identifier(), 5);
                // |26-0| Fixed Point Floating point (17,10) no sign bit representing the resistance
                let resistance_fixed_point = FixedU32::<U10>::from_num(*resistance);
                write_sequence.write_bits(resistance_fixed_point.to_bits().into(), 27);

                self.spi_device.write(write_sequence.as_bytes())?;
                Ok(())
            }
        }
    }

    //check if the channel is configured
    pub fn channel_enabled(&mut self, channel: &LTC2983Channel) -> bool {
        let mut read_sequence = ByteBuffer::new();
        read_sequence.write_u8(LTC2983_READ);
        read_sequence.write_u16(channel.start_address());
        read_sequence.write_u8(0); //Dummy Data for read

        let mut recv: [u8; 4] = [0, 0, 0, 0];
        match self.spi_device.transfer(&mut recv, read_sequence.as_bytes()) {
            Ok(_) => {
                //if the upper 5bits of the channel are zero, then the channel is disabled so checking for not zero means the channel is enabled
                if recv[3] & 0xf8 != 0 {
                    true
                } else {
                    false
                }
            }
            Err(_err) => {
                //on communication error assume unconfigured channel
                false
            }
        }
    }

    pub fn start_conversion(&mut self, channel: &LTC2983Channel) -> Result<(), LTC2983Error<SPI::Error>> {
        //start measurement
        let mut start_command_bytes = ByteBuffer::new();
        start_command_bytes.write_u8(LTC2983_WRITE);
        start_command_bytes.write_u16(STATUS_REGISTER);
        start_command_bytes.write_bits(0x4, 3);
        start_command_bytes.write_bits(channel.identifier(), 5);

        self.spi_device.write(start_command_bytes.as_bytes())?;

        Ok(())
    }

    pub fn start_multi_conversion(&mut self, channels: &Vec<LTC2983Channel>) -> Result<(), LTC2983Error<SPI::Error>> {
        let mut write_channel_mask = ByteBuffer::new();
        let mut mask: u32 = 0x0;
        for chan in channels {
            mask |= chan.mask();
        }
        write_channel_mask.write_u8(LTC2983_WRITE);
        write_channel_mask.write_u16(MULTI_CHANNEL_MASK_REGISTER);
        write_channel_mask.write_u32(mask);
        self.spi_device.write(write_channel_mask.as_bytes())?;

        let mut start_multi_conversion_bytes = ByteBuffer::new();
        start_multi_conversion_bytes.write_u8(LTC2983_WRITE);
        start_multi_conversion_bytes.write_u16(STATUS_REGISTER);
        start_multi_conversion_bytes.write_bits(0x4, 3);
        start_multi_conversion_bytes.write_bits(0x0, 5);

        self.spi_device.write(start_multi_conversion_bytes.as_bytes())?;
        Ok(())
    }

    pub fn read_temperature(&mut self, channel: &LTC2983Channel) -> Result<LTC2983Result, LTC2983Error<SPI::Error>> {
        let mut read_temperature_bytes = ByteBuffer::new();
        read_temperature_bytes.write_u8(LTC2983_READ);
        read_temperature_bytes.write_u16(channel.result_address());
        read_temperature_bytes.write_u32(0x0); //Dummy bytes for reading

        let mut recv: [u8; 7] = [0, 0, 0, 0, 0, 0, 0];
        self.spi_device.transfer(&mut recv, read_temperature_bytes.as_bytes())?;

        Ok(LTC2983Result::from([recv[3], recv[4], recv[5], recv[6]]))
    }

    pub fn read_multi_temperature(&mut self, channels: &Vec<LTC2983Channel>) -> Vec<Result<LTC2983Result, LTC2983Error<SPI::Error>>> {
        channels.iter().map(|chan| {
            self.read_temperature(chan)
        }).collect()
    }

    ///do multiple rounds of conversion for a channel then calculate the average of the temperatures read out
    pub fn get_temperature_avg(&mut self, channel: &LTC2983Channel, rounds: usize) -> Result<f32, LTC2983Error<SPI::Error>> {
        let mut values = Vec::new();
        let mut r = 0;

        while r < rounds {
            self.start_conversion(channel)?;
            
            for i in 1..(3+rounds) {
                println!("{:?}",self.status().unwrap());
                
                if !self.status()?.done() {
                    thread::sleep(Duration::from_millis(100));
                }
            }

            if !self.status()?.done() {
                break;
            }


                         
            let mut was_error = false;
            let mut v: f32 = 0.;
            match self.read_temperature(channel) {
                Ok(ltc_res) => {
                    match ltc_res {
                        LTC2983Result::Invalid(_) | LTC2983Result::Suspect(_, _) => {
                            was_error = true;
                        },
                        LTC2983Result::Valid(temp) => {
                            v = temp;
                        }
                    }
                },
                Err(_err) => {
                    was_error = true;
                },
            }

                
            if !was_error {
                values.push(v);
                r += 1;
            }
        }

        values.into_iter().reduce(|acc, e| acc + e).and_then(|v| Some(v / ( rounds as f32))).ok_or(LTC2983Error::AvgCalculationError)
    }

    ///do multiple rounds of conversion for multiple channels then calculate the average of the temperatures read out
    pub fn get_multi_temperature_avg(&mut self, channels: &Vec<LTC2983Channel>, rounds: usize) -> Result<Vec<f32>, LTC2983Error<SPI::Error>> {
        let mut values = Vec::new();
        let mut r = 0;

        while r < rounds {
            self.start_multi_conversion(channels)?;
            while !self.status()?.done {}
            let mut v = Vec::new();
            let mut was_error = false;
            for res in self.read_multi_temperature(channels) {
                match res {
                    Ok(ltc_res) => {
                        match ltc_res {
                            LTC2983Result::Invalid(_) | LTC2983Result::Suspect(_, _) => {
                                was_error = true;
                            },
                            LTC2983Result::Valid(temp) => {
                                v.push(temp);
                            }
                        }
                    },
                    Err(_err) => {
                        was_error = true;
                    },
                }
            }
            if !was_error {
                values.push(v);
                r += 1;
            }
        }

        values.into_iter().reduce(|acc, e| {
            acc.iter().zip(e.iter()).map(|(&a, &b)| a+b).collect::<Vec<f32>>() // do a component wise add of the values
        }).and_then(|v| {
            Some(v.iter().map(|x| x/(rounds as f32)).collect()) // calculate average by dividing by the amount of values captured
        }).ok_or(LTC2983Error::AvgCalculationError)
    }
}

fn reformat_fixedf24_to_fixed_f32(bytes_f24: &[u8; 3]) -> [u8; 4]{
    if bytes_f24[0] & 0x80 == 0x80 {
        [0xff, bytes_f24[0], bytes_f24[1], bytes_f24[2]]
    } else {
        [0x00, bytes_f24[0], bytes_f24[1], bytes_f24[2]]
    }
}

#[cfg(test)]
mod tests {
    use fixed::{FixedI32, types::extra::U10};

    use super::*;

    #[test]
    fn test_fixedf24_u10_to_f32_signed() {
        let bytes: [u8; 3] = [ 0x7f, 0xff, 0xff ];
        let value = FixedI32::<U10>::from_be_bytes(reformat_fixedf24_to_fixed_f32(&bytes));
        assert!(value.to_num::<f32>() - (8191.999 as f32) < 1./1024.); // error should be smaller than smallest fixed point value 1./1024.

        let bytes: [u8; 3] = [ 0x10, 0x00, 0x00 ];
        let value = FixedI32::<U10>::from_be_bytes(reformat_fixedf24_to_fixed_f32(&bytes));
        assert!(value.to_num::<f32>() - (1024 as f32) < 1./1024.); // error should be smaller than smallest fixed point value 1./1024.

        let bytes: [u8; 3] = [ 0x00, 0x04, 0x00 ];
        let value = FixedI32::<U10>::from_be_bytes(reformat_fixedf24_to_fixed_f32(&bytes));
        assert!(value.to_num::<f32>() - (1 as f32) < 1./1024.); // error should be smaller than smallest fixed point value 1./1024.

        let bytes: [u8; 3] = [ 0x00, 0x00, 0x01 ];
        let value = FixedI32::<U10>::from_be_bytes(reformat_fixedf24_to_fixed_f32(&bytes));
        assert!(value.to_num::<f32>() - (1./1024. as f32) < 1./1024.); // error should be smaller than smallest fixed point value 1./1024.

        let bytes: [u8; 3] = [ 0x00, 0x00, 0x00 ];
        let value = FixedI32::<U10>::from_be_bytes(reformat_fixedf24_to_fixed_f32(&bytes));
        assert!(value.to_num::<f32>() - (0. as f32) < 1./1024.); // error should be smaller than smallest fixed point value 1./1024.

        let bytes: [u8; 3] = [ 0xff, 0xff, 0xff ];
        let value = FixedI32::<U10>::from_be_bytes(reformat_fixedf24_to_fixed_f32(&bytes));
        assert!(value.to_num::<f32>() - (-1./1024. as f32) < 1./1024.); // error should be smaller than smallest fixed point value 1./1024.

        let bytes: [u8; 3] = [ 0xff, 0xfc, 0x00 ];
        let value = FixedI32::<U10>::from_be_bytes(reformat_fixedf24_to_fixed_f32(&bytes));
        assert!(value.to_num::<f32>() - (-1 as f32) < 1./1024.); // error should be smaller than smallest fixed point value 1./1024.

        let bytes: [u8; 3] = [ 0xfb, 0xbb, 0x67 ];
        let value = FixedI32::<U10>::from_be_bytes(reformat_fixedf24_to_fixed_f32(&bytes));
        assert!(value.to_num::<f32>() - (-273.15 as f32) < 1./1024.); // error should be smaller than smallest fixed point value 1./1024.

        let bytes: [u8; 3] = [ 0xf8, 0xd1, 0x52 ];
        let value = FixedI32::<U10>::from_be_bytes(reformat_fixedf24_to_fixed_f32(&bytes));
        assert!(value.to_num::<f32>() - (-459.67 as f32) < 1./1027.); // error should be smaller than smallest fixed point value 1./1024.
    }
}
