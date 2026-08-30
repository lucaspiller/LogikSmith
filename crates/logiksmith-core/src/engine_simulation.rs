impl Engine {
    fn validate_and_build_snapshots(
    &self,
    inputs: &[SimulationInput],
) -> Result<Vec<InputSnapshot>, SimulationError> {
    let mut supplied: Vec<Option<&SimulationInput>> = vec![None; self.config.endpoints.len()];
    for input in inputs {
        let Some(index) = self
            .config
            .endpoints
            .iter()
            .position(|endpoint| endpoint.name == input.endpoint)
        else {
            return Err(SimulationError::UnknownEndpoint(input.endpoint.clone()));
        };
        let endpoint = &self.config.endpoints[index];
        if endpoint.direction != EndpointDirection::Input {
            return Err(SimulationError::EndpointNotInput {
                endpoint: input.endpoint.clone(),
                actual: endpoint.direction,
            });
        }
        if supplied[index].is_some() {
            return Err(SimulationError::DuplicateInput(input.endpoint.clone()));
        }
        validate_simulation_input(endpoint, input)?;
        supplied[index] = Some(input);
    }
    self.config
        .endpoints
        .iter()
        .enumerate()
        .filter_map(|(index, endpoint)| {
            (endpoint.direction == EndpointDirection::Input).then(|| {
                supplied[index]
                    .ok_or_else(|| SimulationError::MissingInput(endpoint.name.clone()))
                    .map(|input| InputSnapshot {
                        endpoint: endpoint.name.clone(),
                        dpt: endpoint.dpt,
                        value: input.value,
                        valid: input.valid,
                        age_ms: input.age_ms,
                    })
            })
        })
        .collect()
    }
}
