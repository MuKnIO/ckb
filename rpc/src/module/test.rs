use crate::error::RPCError;
use ckb_app_config::BlockAssemblerConfig;
use ckb_chain::chain::ChainController;
use ckb_dao::DaoCalculator;
use ckb_jsonrpc_types::{
    AsEpochNumberWithFraction, Block, BlockTemplate, Byte32, JsonBytes, Script, Transaction,
};
use ckb_logger::error;
use ckb_network::{NetworkController, SupportProtocols};
use ckb_shared::{shared::Shared, Snapshot};
use ckb_store::ChainStore;
use ckb_types::{
    core::{
        self,
        cell::{
            resolve_transaction_with_options, OverlayCellProvider, ResolveOptions,
            ResolvedTransaction, TransactionsProvider,
        },
        BlockView,
    },
    packed,
    prelude::*,
    H256,
};
use ckb_verification_traits::Switch;
use jsonrpc_core::Result;
use jsonrpc_derive::rpc;
use std::collections::HashSet;
use std::sync::Arc;

#[rpc(server)]
#[doc(hidden)]
pub trait IntegrationTestRpc {
    #[rpc(name = "process_block_without_verify")]
    fn process_block_without_verify(&self, data: Block, broadcast: bool) -> Result<Option<H256>>;

    #[rpc(name = "truncate")]
    fn truncate(&self, target_tip_hash: H256) -> Result<()>;

    #[rpc(name = "generate_block")]
    fn generate_block(
        &self,
        block_assembler_script: Option<Script>,
        block_assembler_message: Option<JsonBytes>,
    ) -> Result<H256>;

    #[rpc(name = "notify_transaction")]
    fn notify_transaction(&self, transaction: Transaction) -> Result<H256>;

    #[rpc(name = "generate_block_with_template")]
    fn generate_block_with_template(&self, block_template: BlockTemplate) -> Result<H256>;

    #[rpc(name = "calculate_dao_field")]
    fn calculate_dao_field(&self, block_template: BlockTemplate) -> Result<Byte32>;
}

pub(crate) struct IntegrationTestRpcImpl {
    pub network_controller: NetworkController,
    pub shared: Shared,
    pub chain: ChainController,
}

impl IntegrationTestRpc for IntegrationTestRpcImpl {
    fn process_block_without_verify(&self, data: Block, broadcast: bool) -> Result<Option<H256>> {
        let block: packed::Block = data.into();
        let block: Arc<BlockView> = Arc::new(block.into_view());
        let ret = self
            .chain
            .internal_process_block(Arc::clone(&block), Switch::DISABLE_ALL);

        if broadcast {
            let content = packed::CompactBlock::build_from_block(&block, &HashSet::new());
            let message = packed::RelayMessage::new_builder().set(content).build();
            if let Err(err) = self
                .network_controller
                .quick_broadcast(SupportProtocols::Relay.protocol_id(), message.as_bytes())
            {
                error!("Broadcast new block failed: {:?}", err);
            }
        }
        if ret.is_ok() {
            Ok(Some(block.hash().unpack()))
        } else {
            error!("process_block_without_verify error: {:?}", ret);
            Ok(None)
        }
    }

    fn truncate(&self, target_tip_hash: H256) -> Result<()> {
        let header = {
            let snapshot = self.shared.snapshot();
            let header = snapshot
                .get_block_header(&target_tip_hash.pack())
                .ok_or_else(|| {
                    RPCError::custom(RPCError::Invalid, "block not found".to_string())
                })?;
            if !snapshot.is_main_chain(&header.hash()) {
                return Err(RPCError::custom(
                    RPCError::Invalid,
                    "block not on main chain".to_string(),
                ));
            }
            header
        };

        // Truncate the chain and database
        self.chain
            .truncate(header.hash())
            .map_err(|err| RPCError::custom(RPCError::Invalid, err.to_string()))?;

        // Clear the tx_pool
        let new_snapshot = Arc::clone(&self.shared.snapshot());
        let tx_pool = self.shared.tx_pool_controller();
        tx_pool
            .clear_pool(new_snapshot)
            .map_err(|err| RPCError::custom(RPCError::Invalid, err.to_string()))?;

        Ok(())
    }

    fn generate_block(
        &self,
        block_assembler_script: Option<Script>,
        block_assembler_message: Option<JsonBytes>,
    ) -> Result<H256> {
        let tx_pool = self.shared.tx_pool_controller();
        let snapshot = Arc::clone(&self.shared.snapshot());
        let block_assembler_config = block_assembler_script.map(|script| BlockAssemblerConfig {
            code_hash: script.code_hash,
            hash_type: script.hash_type,
            args: script.args,
            message: block_assembler_message.unwrap_or_default(),
            use_binary_version_as_message_prefix: false,
            binary_version: "TEST".to_string(),
        });
        let block_template = tx_pool
            .get_block_template_with_block_assembler_config(
                None,
                None,
                None,
                snapshot,
                block_assembler_config,
            )
            .map_err(|err| RPCError::custom(RPCError::Invalid, err.to_string()))?
            .map_err(|err| RPCError::custom(RPCError::CKBInternalError, err.to_string()))?;

        self.process_and_announce_block(block_template.into())
    }

    fn notify_transaction(&self, tx: Transaction) -> Result<H256> {
        let tx: packed::Transaction = tx.into();
        let tx: core::TransactionView = tx.into_view();
        let tx_pool = self.shared.tx_pool_controller();
        let tx_hash = tx.hash();
        if let Err(e) = tx_pool.notify_txs(vec![tx]) {
            error!("send notify_txs request error {}", e);
            return Err(RPCError::ckb_internal_error(e));
        }
        Ok(tx_hash.unpack())
    }

    fn generate_block_with_template(&self, block_template: BlockTemplate) -> Result<H256> {
        let dao_field = self.calculate_dao_field(block_template.clone())?;

        let mut update_dao_template = block_template;
        update_dao_template.dao = dao_field;
        let block = update_dao_template.into();
        self.process_and_announce_block(block)
    }

    fn calculate_dao_field(&self, block_template: BlockTemplate) -> Result<Byte32> {
        let snapshot: &Snapshot = &self.shared.snapshot();
        let consensus = snapshot.consensus();
        let parent_header = snapshot
            .get_block_header(&block_template.parent_hash.pack())
            .expect("parent header should be stored");
        let mut seen_inputs = HashSet::new();

        let txs: Vec<_> = packed::Block::from(block_template.clone())
            .transactions()
            .into_iter()
            .map(|tx| tx.into_view())
            .collect();

        let transactions_provider = TransactionsProvider::new(txs.as_slice().iter());
        let overlay_cell_provider = OverlayCellProvider::new(&transactions_provider, snapshot);
        let resolve_opts = {
            let epoch_number = block_template.epoch.epoch_number();
            let hardfork_switch = consensus.hardfork_switch();
            ResolveOptions::new().apply_current_features(hardfork_switch, epoch_number)
        };

        let rtxs = txs
            .iter()
            .map(|tx| {
                resolve_transaction_with_options(
                    tx.clone(),
                    &mut seen_inputs,
                    &overlay_cell_provider,
                    snapshot,
                    resolve_opts,
                )
                .map_err(|err| {
                    error!(
                        "resolve transactions error when generating block \
                         with block template, error: {:?}",
                        err
                    );
                    RPCError::invalid_params(err.to_string())
                })
            })
            .collect::<Result<Vec<ResolvedTransaction>>>()?;

        Ok(DaoCalculator::new(consensus, &snapshot.as_data_provider())
            .dao_field(&rtxs, &parent_header)
            .expect("dao calculation should be OK")
            .into())
    }
}

impl IntegrationTestRpcImpl {
    fn process_and_announce_block(&self, block: packed::Block) -> Result<H256> {
        let block_view = Arc::new(block.into_view());
        let content = packed::CompactBlock::build_from_block(&block_view, &HashSet::new());
        let message = packed::RelayMessage::new_builder().set(content).build();

        // insert block to chain
        self.chain
            .process_block(Arc::clone(&block_view))
            .map_err(|err| RPCError::custom(RPCError::CKBInternalError, err.to_string()))?;

        // announce new block
        if let Err(err) = self
            .network_controller
            .quick_broadcast(SupportProtocols::Relay.protocol_id(), message.as_bytes())
        {
            error!("Broadcast new block failed: {:?}", err);
        }

        Ok(block_view.header().hash().unpack())
    }
}
