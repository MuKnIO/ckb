use crate::error::RPCError;
use ckb_chain::chain::ChainController;
use ckb_jsonrpc_types::{Block, BlockTemplate, Uint64, Version};
use ckb_logger::{debug, error, info};
use ckb_network::{NetworkController, SupportProtocols};
use ckb_shared::{shared::Shared, Snapshot};
use ckb_types::{core, packed, prelude::*, H256};
use ckb_verification::HeaderVerifier;
use ckb_verification_traits::Verifier;
use faketime::unix_time_as_millis;
use jsonrpc_core::{Error, Result};
use jsonrpc_derive::rpc;
use std::collections::HashSet;
use std::fmt::Debug;
use std::sync::Arc;

/// RPC Module Miner for miners.
///
/// A miner gets a template from CKB, optionally selects transactions, resolves the PoW puzzle, and
/// submits the found new block.
#[rpc(server)]
pub trait MinerRpc {
    /// Returns block template for miners.
    ///
    /// Miners can assemble the new block from the template. The RPC is designed to allow miners
    /// to remove transactions and adding new transactions to the block.
    ///
    /// ## Params
    ///
    /// * `bytes_limit` - the max serialization size in bytes of the block.
    ///     (**Optional:** the default is the consensus limit.)
    /// * `proposals_limit` - the max count of proposals.
    ///     (**Optional:** the default is the consensus limit.)
    /// * `max_version` - the max block version.
    ///     (**Optional:** the default is one configured in the current client version.)
    ///
    /// ## Examples
    ///
    /// Request
    ///
    /// ```json
    /// {
    ///   "id": 42,
    ///   "jsonrpc": "2.0",
    ///   "method": "get_block_template",
    ///   "params": [
    ///     null,
    ///     null,
    ///     null
    ///   ]
    /// }
    /// ```
    ///
    /// Response
    ///
    /// ```json
    /// {
    ///   "id": 42,
    ///   "jsonrpc": "2.0",
    ///   "result": {
    ///     "bytes_limit": "0x91c08",
    ///     "cellbase": {
    ///       "cycles": null,
    ///       "data": {
    ///         "cell_deps": [],
    ///         "header_deps": [],
    ///         "inputs": [
    ///           {
    ///             "previous_output": {
    ///               "index": "0xffffffff",
    ///               "tx_hash": "0x0000000000000000000000000000000000000000000000000000000000000000"
    ///             },
    ///             "since": "0x401"
    ///           }
    ///         ],
    ///         "outputs": [
    ///           {
    ///             "capacity": "0x18e64efc04",
    ///             "lock": {
    ///               "code_hash": "0x28e83a1277d48add8e72fadaa9248559e1b632bab2bd60b27955ebc4c03800a5",
    ///               "hash_type": "data",
    ///               "args": "0x"
    ///             },
    ///             "type": null
    ///           }
    ///         ],
    ///         "outputs_data": [
    ///           "0x"
    ///         ],
    ///         "version": "0x0",
    ///         "witnesses": [
    ///           "0x650000000c00000055000000490000001000000030000000310000001892ea40d82b53c678ff88312450bbb17e164d7a3e0a90941aa58839f56f8df20114000000b2e61ff569acf041b3c2c17724e2379c581eeac30c00000054455354206d657373616765"
    ///         ]
    ///       },
    ///       "hash": "0xbaf7e4db2fd002f19a597ca1a31dfe8cfe26ed8cebc91f52b75b16a7a5ec8bab"
    ///     },
    ///     "compact_target": "0x1e083126",
    ///     "current_time": "0x174c45e17a3",
    ///     "cycles_limit": "0xd09dc300",
    ///     "dao": "0xd495a106684401001e47c0ae1d5930009449d26e32380000000721efd0030000",
    ///     "epoch": "0x7080019000001",
    ///     "extension": null,
    ///     "number": "0x401",
    ///     "parent_hash": "0xa5f5c85987a15de25661e5a214f2c1449cd803f071acc7999820f25246471f40",
    ///     "proposals": ["0xa0ef4eb5f4ceeb08a4c8"],
    ///     "transactions": [],
    ///     "uncles": [
    ///       {
    ///         "hash": "0xdca341a42890536551f99357612cef7148ed471e3b6419d0844a4e400be6ee94",
    ///         "header": {
    ///           "compact_target": "0x1e083126",
    ///           "dao": "0xb5a3e047474401001bc476b9ee573000c0c387962a38000000febffacf030000",
    ///           "epoch": "0x7080018000001",
    ///           "extra_hash": "0x0000000000000000000000000000000000000000000000000000000000000000",
    ///           "nonce": "0x0",
    ///           "number": "0x400",
    ///           "parent_hash": "0xae003585fa15309b30b31aed3dcf385e9472c3c3e93746a6c4540629a6a1ed2d",
    ///           "proposals_hash": "0x0000000000000000000000000000000000000000000000000000000000000000",
    ///           "timestamp": "0x5cd2b118",
    ///           "transactions_root": "0xc47d5b78b3c4c4c853e2a32810818940d0ee403423bea9ec7b8e566d9595206c",
    ///           "version":"0x0"
    ///         },
    ///         "proposals": [],
    ///         "required": false
    ///       }
    ///     ],
    ///     "uncles_count_limit": "0x2",
    ///     "version": "0x0",
    ///     "work_id": "0x0"
    ///   }
    /// }
    /// ```
    #[rpc(name = "get_block_template")]
    fn get_block_template(
        &self,
        bytes_limit: Option<Uint64>,
        proposals_limit: Option<Uint64>,
        max_version: Option<Version>,
    ) -> Result<BlockTemplate>;

    /// Submit new block to the network.
    ///
    /// ## Params
    ///
    /// * `work_id` - The same work ID returned from [`get_block_template`](#tymethod.get_block_template).
    /// * `block` - The assembled block from the block template and which PoW puzzle has been resolved.
    ///
    /// ## Examples
    ///
    /// Request
    ///
    /// ```json
    /// {
    ///   "id": 42,
    ///   "jsonrpc": "2.0",
    ///   "method": "submit_block",
    ///   "params": [
    ///     "example",
    ///     {
    ///       "header": {
    ///         "compact_target": "0x1e083126",
    ///         "dao": "0xb5a3e047474401001bc476b9ee573000c0c387962a38000000febffacf030000",
    ///         "epoch": "0x7080018000001",
    ///         "extra_hash": "0x0000000000000000000000000000000000000000000000000000000000000000",
    ///         "nonce": "0x0",
    ///         "number": "0x400",
    ///         "parent_hash": "0xae003585fa15309b30b31aed3dcf385e9472c3c3e93746a6c4540629a6a1ed2d",
    ///         "proposals_hash": "0x0000000000000000000000000000000000000000000000000000000000000000",
    ///         "timestamp": "0x5cd2b117",
    ///         "transactions_root": "0xc47d5b78b3c4c4c853e2a32810818940d0ee403423bea9ec7b8e566d9595206c",
    ///         "version": "0x0"
    ///       },
    ///       "proposals": [],
    ///       "transactions": [
    ///         {
    ///           "cell_deps": [],
    ///           "header_deps": [],
    ///           "inputs": [
    ///             {
    ///               "previous_output": {
    ///                 "index": "0xffffffff",
    ///                 "tx_hash": "0x0000000000000000000000000000000000000000000000000000000000000000"
    ///               },
    ///               "since": "0x400"
    ///             }
    ///           ],
    ///           "outputs": [
    ///             {
    ///               "capacity": "0x18e64b61cf",
    ///               "lock": {
    ///                 "code_hash": "0x28e83a1277d48add8e72fadaa9248559e1b632bab2bd60b27955ebc4c03800a5",
    ///                 "hash_type": "data",
    ///                 "args": "0x"
    ///               },
    ///               "type": null
    ///             }
    ///           ],
    ///           "outputs_data": [
    ///             "0x"
    ///           ],
    ///           "version": "0x0",
    ///           "witnesses": [
    ///             "0x450000000c000000410000003500000010000000300000003100000028e83a1277d48add8e72fadaa9248559e1b632bab2bd60b27955ebc4c03800a5000000000000000000"
    ///           ]
    ///         }
    ///       ],
    ///       "uncles": []
    ///     }
    ///   ]
    /// }
    /// ```
    ///
    /// Response
    ///
    /// ```json
    /// {
    ///   "id": 42,
    ///   "jsonrpc": "2.0",
    ///   "result": "0xa5f5c85987a15de25661e5a214f2c1449cd803f071acc7999820f25246471f40"
    /// }
    /// ```
    #[rpc(name = "submit_block")]
    fn submit_block(&self, work_id: String, block: Block) -> Result<H256>;
}

pub(crate) struct MinerRpcImpl {
    pub network_controller: NetworkController,
    pub shared: Shared,
    pub chain: ChainController,
}

impl MinerRpc for MinerRpcImpl {
    fn get_block_template(
        &self,
        bytes_limit: Option<Uint64>,
        proposals_limit: Option<Uint64>,
        max_version: Option<Version>,
    ) -> Result<BlockTemplate> {
        let bytes_limit = match bytes_limit {
            Some(b) => Some(b.into()),
            None => None,
        };

        let proposals_limit = match proposals_limit {
            Some(b) => Some(b.into()),
            None => None,
        };

        // Accessing shared resources
        // -> First we have to discuss how the txpool gets started,
        //    because the miner rpc accesses the tx pool under the hood.
        // -> The txpool is initialized using a TxPoolServiceBuilder.
        // -> This creates a template for two things:
        //    1. the service which maintains the tx pool.
        //    2. the controller of the service.
        //    3. The channel used by the controller to message the service with commands.
        // -> The first thing we do is to initialize the TxPoolServiceBuilder.
        //    This gives us the TxPoolController.
        // -> Once that's initialized, we call the `start` method.
        //    This start method starts the TxPoolService.
        // -> When the TxPoolService is started,
        //    we also configure the listening thread, on the service side
        //    to listen for messages coming from the controller.
        //    This is done by spawning a thread
        //    and using the `process` method to handle messages.
        //
        // What happens when we send an RPC call?
        //
        // 1. The miner calls this RPC endpoint, the functionality below gets triggered.
        // -> Below, the first step is to call get_block_template from the shared resource manager.
        // -> This calls the get_block_template of tx_pool_controller,
        //
        // 2. Accessing the txpool controller resource.
        // -> which accesses the tx pool controller shared resource.
        // -> from there we call get_block_template of the tx pool shared resource.
        // -> This inserts the current snapshot
        // -> Then it calls the tx pool controller method get_block_template
        // -> This is a wrapper which calls get_block_template_with_block_assembler_config
        // -> We then construct a request object,
        // -> And send a message of type BlockTemplate(request) to the underlying
        //    txpool service.
        //    NOTE: In the above section we discussed how service and controller
        //    are initialized. There, the handler for the incoming messages from the controller
        //    also is configured.
        // -> The TxPoolController has a `sender` field, this field binds to the write port of the channel.
        //    NOTE: The TxPoolController is initialized together with the TxPoolService,
        //    in TxPoolServiceBuilder.
        //    It initializes the channel,
        //    and passes the `sender` side to the controller
        //    the `receiver` side to the service.
        //    see the above section on initializing the txpool service.
        // -> We send the messages over the write port (sender),
        //
        // 3. The txpool service receives the message
        // -> On the service side, there is a `process` handler,
        //    which matches the incoming message
        // -> This matches on `Message::BlockTemplate`,
        //    and dispatches a call to the get_block_template
        //    method of the service.
        // -> Next, we are interested in the section
        //    that returns **cellbase**, since this is where we
        //    indicate the transaction pending confirmations to the miner.
        //    This is fetched from the snapshot, via the call:
        //    build_block_template_cellbase(snapshot, <assembler_config>)
        // -> grab the response from sending the message,
        // -> return the block template result.
        self.shared
            .get_block_template(bytes_limit, proposals_limit, max_version.map(Into::into))
            .map_err(|err| {
                error!("send get_block_template request error {}", err);
                RPCError::ckb_internal_error(err)
            })?
            .map_err(|err| {
                error!("get_block_template result error {}", err);
                RPCError::from_any_error(err)
            })
    }

    fn submit_block(&self, work_id: String, block: Block) -> Result<H256> {
        let block: packed::Block = block.into();
        let block: Arc<core::BlockView> = Arc::new(block.into_view());
        let header = block.header();
        debug!(
            "start to submit block, work_id = {}, block = #{}({})",
            work_id,
            block.number(),
            block.hash()
        );

        let snapshot: &Snapshot = &self.shared.snapshot();
        let consensus = snapshot.consensus();

        // Reject block extension for public chain: mainnet and testnet.
        if block.extension().is_some() && consensus.is_public_chain() {
            let err = "the block extension should be null";
            return Err(RPCError::custom_with_error(RPCError::Invalid, err));
        }

        // Verify header
        HeaderVerifier::new(snapshot, consensus)
            .verify(&header)
            .map_err(|err| handle_submit_error(&work_id, &err))?;

        // Verify and insert block
        let is_new = self
            .chain
            .process_block(Arc::clone(&block))
            .map_err(|err| handle_submit_error(&work_id, &err))?;
        info!(
            "end to submit block, work_id = {}, is_new = {}, block = #{}({})",
            work_id,
            is_new,
            block.number(),
            block.hash()
        );

        // Announce only new block
        if is_new {
            debug!(
                "[block_relay] announce new block {} {} {}",
                header.number(),
                header.hash(),
                unix_time_as_millis()
            );
            let content = packed::CompactBlock::build_from_block(&block, &HashSet::new());
            let message = packed::RelayMessage::new_builder().set(content).build();
            let pid = if self.network_controller.load_ckb2021() {
                SupportProtocols::RelayV2.protocol_id()
            } else {
                SupportProtocols::Relay.protocol_id()
            };
            if let Err(err) = self
                .network_controller
                .quick_broadcast(pid, message.as_bytes())
            {
                error!("Broadcast new block failed: {:?}", err);
            }
        }

        Ok(header.hash().unpack())
    }
}

fn handle_submit_error<E: std::fmt::Display + Debug>(work_id: &str, err: &E) -> Error {
    error!("[{}] submit_block error: {:?}", work_id, err);
    RPCError::custom_with_error(RPCError::Invalid, err)
}
