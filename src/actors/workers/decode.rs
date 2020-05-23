// Copyright 2017-2019 Parity Technologies (UK) Ltd.
// This file is part of substrate-archive.

// substrate-archive is free software: you can redistribute it and/or modify
// it under the terms of the GNU General Public License as published by
// the Free Software Foundation, either version 3 of the License, or
// (at your option) any later version.

// substrate-archive is distributed in the hope that it will be useful,
// but WITHOUT ANY WARRANTY; without even the implied warranty of
// MERCHANTABILITY or FITNESS FOR A PARTICULAR PURPOSE.  See the
// GNU General Public License for more details.

// You should have received a copy of the GNU General Public License
// along with substrate-archive.  If not, see <http://www.gnu.org/licenses/>.

//! Actors which do work by decoding data before it's inserted into the database
//! these actors may do highly parallelized work
//! These actors do not make any external connections to a Database or Network

use crate::types::{Block, SignedExtrinsic, Inherent, RawExtrinsic, Substrate, System};
use crate::actors::scheduler::{Algorithm, Scheduler};
use bastion::prelude::*;
use desub::{decoder::Decoder, TypeDetective};

const REDUNDANCY: usize = 64;

/// the main actor
/// holds the internal decoder state
pub fn actor<T, P>(db_workers: ChildrenRef, decoder: Decoder<P>) -> Result<ChildrenRef, ()>
where
    T: Substrate + Send + Sync,
    P: TypeDetective + Send + Sync + 'static,
    <T as System>::BlockNumber: Into<u32>,
{
    // actor that manages decode state, but sends decoding to other actors
    // TODO: could be a supervisor
    // TODO: rework desub so that decoder doesn't need to be cloned everytime we send it to an actor
    // could do a 'stateless' approach that only sends the current spec + metadata to the decoder to decode with
    // rather than keeping all metadata ever presented
    Bastion::children(|children: Children| {
        children
            .with_exec(move |ctx: BastionContext| {
                let workers = db_workers.clone();
                let mut decoder = decoder.clone();
                async move {
                    log::info!("Decode worker started");
                    let mut sched = Scheduler::new(Algorithm::RoundRobin, &ctx);
                    sched.add_worker("db", &workers);
                    loop {
                        msg! {
                            ctx.recv().await?,
                            block: Block<T> =!> {
                                process_block(block.clone(), &mut sched).await;
                                process_extrinsics::<T, P>(decoder.clone(), vec![block], &mut sched).await;
                                answer!(ctx, super::ArchiveAnswer::Success).expect("couldn't answer");
                             };
                             blocks: Vec<Block<T>> =!> {
                                 process_blocks(blocks.clone(), &mut sched).await;
                                 process_extrinsics(decoder.clone(), blocks, &mut sched).await;
                                 answer!(ctx, super::ArchiveAnswer::Success).expect("couldn't answer");
                            };
                            ref broadcast: super::Broadcast => {
                                () // we don't need to do any cleanup
                            };
                            e: _ => log::warn!("Received unknown data {:?}", e);
                        }
                    }
                }
            })
    })
}

pub async fn process_block<T>(block: Block<T>, sched: &mut Scheduler<'_>)
where
    T: Substrate + Send + Sync,
{
    let v = sched.ask_next("db", block).unwrap().await;
    log::debug!("{:?}", v);
}

pub async fn process_blocks<T>(blocks: Vec<Block<T>>, sched: &mut Scheduler<'_>)
where
    T: Substrate + Send + Sync,
{
    log::info!("Processing blocks");
    let v = sched.ask_next("db", blocks).unwrap().await;
    log::debug!("{:?}", v);
}

#[derive(Debug)]
enum ExtrinsicType<T: Substrate + Send + Sync> {
    Signed(SignedExtrinsic<T>),
    NotSigned(Inherent<T>)
}


struct ExtVec<T>(Vec<ExtrinsicType<T>>) where T: Substrate + Send + Sync;
impl<T> ExtVec<T> where T: Substrate + Send + Sync {
    fn split(self) -> (Vec<SignedExtrinsic<T>>, Vec<Inherent<T>>) {
        let s = self.0;
        let (mut signed, mut not_signed) = (Vec::new(), Vec::new());
        for e in s.into_iter() {
            match e {
                ExtrinsicType::Signed(e) => {
                    signed.push(e)
                },
                ExtrinsicType::NotSigned(e) => {
                    not_signed.push(e)
                }
            }
        }
        (signed, not_signed)
    }
}

impl<T> From<Vec<ExtrinsicType<T>>> for ExtVec<T> where T: Substrate + Send + Sync {
    fn from(ext: Vec<ExtrinsicType<T>>) -> ExtVec<T> {
        ExtVec(ext)
    }
}

pub async fn process_extrinsics<T, P>(
    mut decoder: Decoder<P>,
    blocks: Vec<Block<T>>,
    sched: &mut Scheduler<'_>,
) where
    T: Substrate + Send + Sync,
    P: TypeDetective + Send + Sync + 'static,
    <T as System>::BlockNumber: Into<u32>,
{
    blocks
        .iter()
        .for_each(|b| decoder.register_version(b.spec, &b.meta));

    let ext: ExtVec<T> = blocks
        .iter()
        .map(|b| Vec::<RawExtrinsic<T>>::from(b))
        .flatten()
        .map(|e| {
            let ext = decoder
                .decode_extrinsic(e.spec, e.inner.as_slice())
                .expect("decoding extrinsic failed");
            if ext.is_signed() {
                ExtrinsicType::Signed(SignedExtrinsic::new(ext, e.hash, e.index, e.block_num))
            } else {
                ExtrinsicType::NotSigned(Inherent::new(ext, e.hash, e.index, e.block_num))
            }
        })
        .collect::<Vec<ExtrinsicType<T>>>()
        .into();

    let (signed, not_signed) = ext.split();
    log::info!("Decoded {} extrinsics", signed.len() + not_signed.len());

    if signed.len() > 0 {
        let v = sched.ask_next("db", signed).unwrap().await;
        log::debug!("{:?}", v);
    }
    if not_signed.len() > 0 {
        let v = sched.ask_next("db", not_signed).unwrap().await;
    }
}
