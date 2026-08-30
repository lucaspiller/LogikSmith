impl Runtime {
    pub fn limits(&self) -> RuntimeLimits {
        self.limits
    }

    pub fn block_ids(&self) -> Vec<BlockId> {
        self.blocks.iter().map(|block| block.id().clone()).collect()
    }

    pub fn last_accepted_at(&self) -> Option<MonotonicMs> {
        self.last_accepted_at
    }
}
